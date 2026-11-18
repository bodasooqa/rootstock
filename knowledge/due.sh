#!/usr/bin/env bash
# Deadlines of open questions: what is due now, what is overdue.
#
# Every question in the open questions file carries a "Due: …" line. As long
# as nobody reads that line when the deadline arrives, the list only grows, and
# a third of it ends up addressed to stages closed a week ago. The script makes
# the deadline machine-checkable — it matches it against the stage state table
# and the list of archived OpenSpec changes.
#
# Modes:
#   due.sh              questions of the stage "in progress" plus overdue ones (report, exit 0)
#   due.sh 4            the same for the named stage
#   due.sh --overdue    overdue only; exit 1 if there are any — pre-push relies on this
#
# A deadline must name a stage number ("stage 4", "acceptance of stage 1") or a
# change with the stage number in its name (`stage1-first-change`). A prose
# deadline like "the stage where there is more than one user" cannot be
# checked, so it counts as missing — like any harness rule held only by prose.
#
# Paths come from `.rootstock.toml` at the project root: `open_questions_path`
# is the questions file, `stages_path` is the file with the stage state table
# (`| N. Name | state |`, states "in progress", "closed"; a row may cover a
# range "4–8. …"). The changes archive is `openspec/changes/archive`.
set -euo pipefail

root=$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)
answers="$root/.rootstock.toml"

mode=${1:-}
case "$mode" in
  --overdue | "") ;;
  --help | -h)
    sed -n '2,23p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    case "$mode" in
      [0-9]*) ;;
      *)
        printf 'due.sh: unknown argument %s. Expected a stage number, --overdue or nothing.\n' "$mode" >&2
        exit 2
        ;;
    esac
    ;;
esac

# Value of a key from the `[answers]` table of the template record:
# `key = "value"`, quotes stripped. Escapes are not expanded: paths have none.
answer() {
  local key=$1 value
  value=$(awk -v key="$key" -v q="'" '
    /^[ \t]*\[/ { inside = ($0 ~ /^[ \t]*\[answers\][ \t]*$/); next }
    inside && $0 ~ ("^" key "[ \t]*=") {
      v = substr($0, index($0, "=") + 1)
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
    printf 'due.sh: %s has no key %s\n' "$answers" "$key" >&2
    exit 1
  fi
  value=${value#./}
  value=${value%/}
  printf '%s\n' "$value"
}

if [[ ! -f "$answers" ]]; then
  printf 'due.sh: no template answers file %s\n' "$answers" >&2
  exit 1
fi

questions=$(answer open_questions_path)
stages=$(answer stages_path)

cd "$root"

perl -CSD -Mutf8 -e '
  my ($file, $stages, $archive, $mode) = @ARGV;

  # --- Stage states: the table "| N. Name | state |" -------------------------
  # One table row may cover a range ("4–8. The rest").
  my %state;
  open my $rf, "<:encoding(UTF-8)", $stages or die "cannot read $stages: $!\n";
  while (my $line = <$rf>) {
    next unless $line =~ /^\|\s*(\d+)(?:\s*[\x{2013}\x{2014}-]\s*(\d+))?\.\s*[^|]*\|\s*([^|]+?)\s*\|/;
    my ($from, $to, $st) = ($1, $2 // $1, $3);
    $state{$_} = $st for ($from .. $to);
  }
  close $rf;
  die "no stage state table found in $stages\n" unless %state;

  my $current;
  for my $n (sort { $a <=> $b } keys %state) {
    $current = $n if $state{$n} =~ /\bin progress\b/i;
  }

  # --- Archived changes: directory name without the leading date --------------
  my %archived;
  if (opendir my $dh, $archive) {
    while (my $d = readdir $dh) {
      $archived{$1} = 1 if $d =~ /^\d{4}-\d{2}-\d{2}-(.+)$/;
    }
    closedir $dh;
  }

  # --- Parsing questions ------------------------------------------------------
  open my $fh, "<:encoding(UTF-8)", $file or die "cannot read $file: $!\n";
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
        ($status, $why) = ("overdue", "change $change is archived");
      } elsif (($state{$n} // "") =~ /\bclosed\b/i) {
        ($status, $why) = ("overdue", "stage $n is closed");
      } elsif (($state{$n} // "") =~ /\bin progress\b/i) {
        ($status, $why) = ("due", "change $change is not archived yet");
      } else {
        ($status, $why) = ("ahead", "stage $n has not started");
      }
    } elsif ($tail =~ /\bstage\s+(\d+)/i) {
      my $n = $1;
      $stage = $n;
      my $st = $state{$n} // "unknown";
      if ($st =~ /\bclosed\b/i)         { ($status, $why) = ("overdue", "stage $n is closed") }
      elsif ($st =~ /\bin progress\b/i) { ($status, $why) = ("due",     "stage $n is in progress") }
      else                              { ($status, $why) = ("ahead",   "stage $n has not started") }
    } else {
      ($status, $why) = ("overdue", "deadline does not name a stage");
    }
    return ($status, $why, $stage);
  };

  my $flush = sub {
    return unless defined $title;
    push @items, { title => $title, line => $tline, due => "no deadline",
                   status => "overdue", why => "no \"Due:\" line" }
      unless $found;
    ($title, $found) = (undef, 0);
  };

  # A code block is a format example in the head of the file, not a question:
  # its lines are not parsed.
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
      # A struck-through heading means the question is closed;
      # archive-questions.sh moves the section to the archive.
      ($title, $tline, $found) = ($h =~ /^~~/ ? undef : $h, $i + 1, 0);
      next;
    }
    next unless defined $title;
    next unless $l =~ /\bDue:\s*(.*)$/;
    my $tail = $1;
    for (my $j = $i + 1; $j < @lines && $lines[$j] =~ /\S/ && $lines[$j] !~ /^(?:##|\s*\d+\.)/; $j++) {
      my $cont = $lines[$j];
      $cont =~ s/^\s+//;
      $tail .= " " . $cont;
    }
    # A deadline is one sentence. The rest of the paragraph is taken only to
    # reassemble a wrapped line; the next sentence already talks about the
    # question, and a stage number in it is not a deadline.
    $tail =~ s/(?<=\.)\s.*$//s;
    my ($status, $why, $stage) = $classify->($tail);
    push @items, { title => $title, line => $i + 1, due => $tail,
                   status => $status, why => $why, stage => $stage };
    $found = 1;
  }
  $flush->();

  # --- Output -----------------------------------------------------------------
  my $short = sub {
    my ($s) = @_;
    $s =~ s/\s+/ /g;
    return length($s) > 96 ? substr($s, 0, 95) . "\x{2026}" : $s;
  };
  my $show = sub {
    my ($it) = @_;
    my $due = $it->{due} eq "no deadline" ? $it->{due} : "Due: " . $it->{due};
    printf "  %s:%d · %s\n      %s · %s\n",
      $file, $it->{line}, $it->{title}, $it->{why}, $short->($due);
  };

  my @overdue = grep { $_->{status} eq "overdue" } @items;

  if ($mode eq "--overdue") {
    unless (@overdue) {
      print "No overdue questions.\n";
      exit 0;
    }
    printf "Overdue questions: %d\n", scalar @overdue;
    $show->($_) for @overdue;
    print "\nEither move the deadline explicitly (a new \"Due:\" line with the reason) or close the question.\nProcedure: the stage-grooming skill.\n";
    exit 1;
  }

  my $stage = $mode ne "" ? $mode : $current;
  unless (defined $stage) {
    die "no stage given, and $stages has no stage \"in progress\"\n";
  }

  printf "Open questions for stage %s.\n\n", $stage;

  my @now = grep { defined $_->{stage} && $_->{stage} == $stage } @items;
  if (@now) {
    printf "Due now: %d\n", scalar @now;
    $show->($_) for @now;
  } else {
    print "No questions are addressed to this stage.\n";
  }

  if (@overdue) {
    printf "\nOverdue: %d\n", scalar @overdue;
    $show->($_) for @overdue;
  }

  my @ahead = grep { $_->{status} eq "ahead" && !(defined $_->{stage} && $_->{stage} == $stage) } @items;
  if (@ahead) {
    my %by;
    $by{$_->{stage}}++ for @ahead;
    printf "\nAhead: %d (stages %s)\n", scalar @ahead, join(", ", sort { $a <=> $b } keys %by);
  }
  exit 0;
' "$questions" "$stages" openspec/changes/archive "$mode"
