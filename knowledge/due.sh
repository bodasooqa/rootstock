#!/usr/bin/env bash
# Сроки открытых вопросов: что наступило, что просрочено.
#
# Каждый вопрос в файле открытых вопросов несёт строку «Нужно к: …». Пока эту
# строку никто не читает в момент наступления срока, список растёт в одну
# сторону, и треть его оказывается адресована этапам, закрытым неделю назад.
# Скрипт делает срок машинным — сопоставляет его с таблицей состояния этапов
# и со списком заархивированных changes OpenSpec.
#
# Режимы:
#   due.sh              вопросы этапа «в работе» плюс просрочка (отчёт, код 0)
#   due.sh 4            то же для названного этапа
#   due.sh --overdue    только просрочка; код 1, если она есть — этим живёт pre-push
#
# Срок обязан называть номер этапа («этапу 4», «приёмке этапа 1») либо change
# с номером этапа в имени (`stage1-first-change`). Прозаический срок вроде
# «этапу, где пользователей станет больше одного» проверить нечем, поэтому он
# считается отсутствующим — как и любое правило обвязки, которое держится
# только прозой.
#
# Пути берутся из `.copier-answers.yml` в корне проекта: `open_questions_path` —
# файл вопросов, `stages_path` — файл с таблицей состояния этапов
# (`| N. Название | состояние |`, состояния «в работе», «закрыт»; строка может
# покрывать диапазон «4–8. …»). Архив changes — `openspec/changes/archive`.
set -euo pipefail

root=$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)
answers="$root/.copier-answers.yml"

mode=${1:-}
case "$mode" in
  --overdue | "") ;;
  --help | -h)
    sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    case "$mode" in
      [0-9]*) ;;
      *)
        printf 'due.sh: непонятный аргумент %s. Ожидается номер этапа, --overdue или ничего.\n' "$mode" >&2
        exit 2
        ;;
    esac
    ;;
esac

# Значение ключа верхнего уровня из файла ответов: `ключ: значение`, кавычки снимаются.
answer() {
  local key=$1 value
  value=$(awk -v key="$key" -v q="'" '
    index($0, key ":") == 1 {
      v = substr($0, length(key) + 2)
      sub(/^[ \t]+/, "", v)
      sub(/[ \t]+$/, "", v)
      f = substr(v, 1, 1)
      if (length(v) >= 2 && (f == "\"" || f == q) && substr(v, length(v), 1) == f)
        v = substr(v, 2, length(v) - 2)
      print v
      exit
    }
  ' "$answers")
  if [[ -z "$value" ]]; then
    printf 'due.sh: в %s нет ключа %s\n' "$answers" "$key" >&2
    exit 1
  fi
  value=${value#./}
  value=${value%/}
  printf '%s\n' "$value"
}

if [[ ! -f "$answers" ]]; then
  printf 'due.sh: нет файла ответов шаблона %s\n' "$answers" >&2
  exit 1
fi

questions=$(answer open_questions_path)
stages=$(answer stages_path)

cd "$root"

perl -CSD -Mutf8 -e '
  my ($file, $stages, $archive, $mode) = @ARGV;

  # --- Состояние этапов: таблица «| N. Название | состояние |» ----------------
  # Одна строка таблицы может покрывать диапазон («4–8. Остальное»).
  my %state;
  open my $rf, "<:encoding(UTF-8)", $stages or die "не читается $stages: $!\n";
  while (my $line = <$rf>) {
    next unless $line =~ /^\|\s*(\d+)(?:\s*[\x{2013}\x{2014}-]\s*(\d+))?\.\s*[^|]*\|\s*([^|]+?)\s*\|/;
    my ($from, $to, $st) = ($1, $2 // $1, $3);
    $state{$_} = $st for ($from .. $to);
  }
  close $rf;
  die "в $stages не нашлось таблицы состояния этапов\n" unless %state;

  my $current;
  for my $n (sort { $a <=> $b } keys %state) {
    $current = $n if $state{$n} =~ /в работе/;
  }

  # --- Заархивированные changes: имя без даты в начале каталога ----------------
  my %archived;
  if (opendir my $dh, $archive) {
    while (my $d = readdir $dh) {
      $archived{$1} = 1 if $d =~ /^\d{4}-\d{2}-\d{2}-(.+)$/;
    }
    closedir $dh;
  }

  # --- Разбор вопросов --------------------------------------------------------
  open my $fh, "<:encoding(UTF-8)", $file or die "не читается $file: $!\n";
  my @lines = split /\n/, do { local $/; <$fh> }, -1;
  close $fh;

  my @items;          # {title, line, due, status, why, stage}
  my ($title, $tline, $found);

  my $classify = sub {
    my ($tail) = @_;
    my ($status, $why, $stage);
    if ($tail =~ /`(stage(\d+)-[a-z0-9._-]+)`/) {
      my ($change, $n) = ($1, $2);
      $stage = $n;
      if ($archived{$change}) {
        ($status, $why) = ("просрочено", "change $change заархивирован");
      } elsif (($state{$n} // "") =~ /закрыт/) {
        ($status, $why) = ("просрочено", "этап $n закрыт");
      } elsif (($state{$n} // "") =~ /в работе/) {
        ($status, $why) = ("наступило", "change $change ещё не заархивирован");
      } else {
        ($status, $why) = ("впереди", "этап $n не начат");
      }
    } elsif ($tail =~ /этап[\x{430}-\x{44F}]*\s+(\d+)/) {
      my $n = $1;
      $stage = $n;
      my $st = $state{$n} // "неизвестен";
      if ($st =~ /закрыт/)      { ($status, $why) = ("просрочено", "этап $n закрыт") }
      elsif ($st =~ /в работе/) { ($status, $why) = ("наступило",  "этап $n в работе") }
      else                      { ($status, $why) = ("впереди",    "этап $n не начат") }
    } else {
      ($status, $why) = ("просрочено", "срок не назван этапом");
    }
    return ($status, $why, $stage);
  };

  my $flush = sub {
    return unless defined $title;
    push @items, { title => $title, line => $tline, due => "срока нет",
                   status => "просрочено", why => "строки «Нужно к:» нет" }
      unless $found;
    ($title, $found) = (undef, 0);
  };

  # Блок кода — пример формата в голове файла, а не вопрос: его строки
  # не разбираются.
  my @plain;
  my $fence = 0;
  for my $l (@lines) {
    if ($l =~ /^\s*```/) { $fence = !$fence; push @plain, ""; next }
    push @plain, $fence ? "" : $l;
  }
  @lines = @plain;

  for (my $i = 0; $i < @lines; $i++) {
    my $l = $lines[$i];
    if ($l =~ /^## (.+?)\s*$/) {
      $flush->();
      my $h = $1;
      # Зачёркнутый заголовок — вопрос закрыт; archive-questions.sh унесёт
      # раздел в архив.
      ($title, $tline, $found) = ($h =~ /^~~/ ? undef : $h, $i + 1, 0);
      next;
    }
    next unless defined $title;
    next unless $l =~ /\x{41D}\x{443}\x{436}\x{43D}\x{43E} \x{43A}:\s*(.*)$/;   # «Нужно к:»
    my $tail = $1;
    for (my $j = $i + 1; $j < @lines && $lines[$j] =~ /\S/ && $lines[$j] !~ /^(?:##|\s*\d+\.)/; $j++) {
      my $cont = $lines[$j];
      $cont =~ s/^\s+//;
      $tail .= " " . $cont;
    }
    # Срок — одно предложение. Продолжение абзаца забирается только затем,
    # чтобы собрать перенесённую строку целиком; следующее предложение уже
    # рассказывает про вопрос, и номер этапа в нём сроком не является.
    $tail =~ s/(?<=\.)\s.*$//s;
    my ($status, $why, $stage) = $classify->($tail);
    push @items, { title => $title, line => $i + 1, due => $tail,
                   status => $status, why => $why, stage => $stage };
    $found = 1;
  }
  $flush->();

  # --- Печать -----------------------------------------------------------------
  my $short = sub {
    my ($s) = @_;
    $s =~ s/\s+/ /g;
    return length($s) > 96 ? substr($s, 0, 95) . "\x{2026}" : $s;
  };
  my $show = sub {
    my ($it) = @_;
    my $due = $it->{due} eq "срока нет" ? $it->{due} : "Нужно к: " . $it->{due};
    printf "  %s:%d · %s\n      %s · %s\n",
      $file, $it->{line}, $it->{title}, $it->{why}, $short->($due);
  };

  my @overdue = grep { $_->{status} eq "просрочено" } @items;

  if ($mode eq "--overdue") {
    unless (@overdue) {
      print "Просроченных вопросов нет.\n";
      exit 0;
    }
    printf "Просрочено вопросов: %d\n", scalar @overdue;
    $show->($_) for @overdue;
    print "\nСрок либо переносится явно (новая строка «Нужно к:» с причиной), либо вопрос закрывается.\nПорядок — скилл stage-grooming.\n";
    exit 1;
  }

  my $stage = $mode ne "" ? $mode : $current;
  unless (defined $stage) {
    die "этап не назван, и в $stages нет этапа «в работе»\n";
  }

  printf "Открытые вопросы к этапу %s.\n\n", $stage;

  my @now = grep { defined $_->{stage} && $_->{stage} == $stage } @items;
  if (@now) {
    printf "Срок наступил: %d\n", scalar @now;
    $show->($_) for @now;
  } else {
    print "К этому этапу вопросов не адресовано.\n";
  }

  if (@overdue) {
    printf "\nПросрочено: %d\n", scalar @overdue;
    $show->($_) for @overdue;
  }

  my @ahead = grep { $_->{status} eq "впереди" && !(defined $_->{stage} && $_->{stage} == $stage) } @items;
  if (@ahead) {
    my %by;
    $by{$_->{stage}}++ for @ahead;
    printf "\nВпереди: %d (этапы %s)\n", scalar @ahead, join(", ", sort { $a <=> $b } keys %by);
  }
  exit 0;
' "$questions" "$stages" openspec/changes/archive "$mode"
