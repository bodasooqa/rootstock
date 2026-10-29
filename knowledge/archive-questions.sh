#!/usr/bin/env bash
# Уносит закрытые вопросы из файла открытых вопросов в архив рядом с ним.
#
# Закрытие вопроса — зачёркивание его заголовка
# (`## ~~Заголовок~~ — закрыт ДД.ММ.ГГГГ`). Дальше скрипт сам переносит раздел
# в архив: открытый документ остаётся списком того, что действительно открыто,
# а вопрос вместе с ответом сохраняется — он объясняет решение лучше, чем
# решение в одиночку.
#
# Режимы:
#   archive-questions.sh           переносит закрытые разделы
#   archive-questions.sh --stage   то же и ставит в индекс оба файла, если
#                                  перенос был, — так зовёт pre-commit: архива
#                                  в индексе обычно нет, и без этого перенос
#                                  делился бы на два коммита
#
# Путь к файлу вопросов — `open_questions_path` из `.copier-answers.yml`; архив —
# тот же путь с суффиксом `-closed` (`open-questions.md` → `open-questions-closed.md`).
set -euo pipefail

root=$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)
answers="$root/.copier-answers.yml"

mode=${1:-}
case "$mode" in
  "" | --stage) ;;
  --help | -h)
    sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    printf 'archive-questions.sh: непонятный аргумент %s. Ожидается --stage или ничего.\n' "$mode" >&2
    exit 2
    ;;
esac

if [[ ! -f "$answers" ]]; then
  printf 'archive-questions.sh: нет файла ответов шаблона %s\n' "$answers" >&2
  exit 1
fi

open=$(awk -v q="'" '
  index($0, "open_questions_path:") == 1 {
    v = substr($0, length("open_questions_path:") + 1)
    sub(/^[ \t]+/, "", v)
    sub(/[ \t]+$/, "", v)
    f = substr(v, 1, 1)
    if (length(v) >= 2 && (f == "\"" || f == q) && substr(v, length(v), 1) == f)
      v = substr(v, 2, length(v) - 2)
    print v
    exit
  }
' "$answers")
if [[ -z "$open" ]]; then
  printf 'archive-questions.sh: в %s нет ключа open_questions_path\n' "$answers" >&2
  exit 1
fi
open=${open#./}
closed="${open%.md}-closed.md"

cd "$root"
[[ -f "$open" ]] || exit 0

moved=$(perl -CSD -Mutf8 -0777 -e '
  my ($open, $closed) = @ARGV;
  open my $fh, "<:encoding(UTF-8)", $open or die "не читается $open: $!\n";
  my $text = do { local $/; <$fh> };
  close $fh;

  # Голова документа — всё до первого раздела; дальше разделы по "## ".
  my ($head, @rest) = split /^(?=## )/m, $text;
  my (@stay, @move);
  for my $s (@rest) {
    if ($s =~ /^## ~~/) { push @move, $s } else { push @stay, $s }
  }
  unless (@move) { print 0; exit 0 }

  # На месте вынутого раздела остаётся его разделитель; двойные схлопываем.
  my $body = $head . join("", @stay);
  $body =~ s/(?:^---\n\n){2,}/---\n\n/mg;
  open my $out, ">:encoding(UTF-8)", $open or die "не пишется $open: $!\n";
  print $out $body;
  close $out;

  my $archive = "";
  if (-e $closed) {
    open my $cf, "<:encoding(UTF-8)", $closed or die "не читается $closed: $!\n";
    $archive = do { local $/; <$cf> };
    close $cf;
  } else {
    my $name = $open;
    $name =~ s{.*/}{};
    $archive = "# Закрытые вопросы\n\n"
      . "Вопросы, на которые ответ уже есть. Перенесены сюда из `$name`\n"
      . "скриптом `knowledge/archive-questions.sh`, как только заголовок зачёркнут.\n"
      . "Хранятся, потому что вопрос вместе с ответом объясняет решение лучше,\n"
      . "чем решение в одиночку.\n\n---\n\n";
  }
  $archive =~ s/\s*\z/\n\n/;
  open my $cf, ">:encoding(UTF-8)", $closed or die "не пишется $closed: $!\n";
  print $cf $archive, join("", @move);
  close $cf;
  print scalar @move;
' "$open" "$closed")

[[ "$moved" -gt 0 ]] || exit 0
printf 'перенесено закрытых вопросов: %d (%s → %s)\n' "$moved" "$open" "$closed"
if [[ "$mode" == --stage ]]; then
  git add -- "$open" "$closed"
fi
