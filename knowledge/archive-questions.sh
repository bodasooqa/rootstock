#!/usr/bin/env bash
# Moves closed questions from the open questions file to an archive next to it.
#
# A question is closed by striking through its heading
# (`## ~~Title~~ — closed YYYY-MM-DD`). The script then moves the section to
# the archive itself: the open document stays a list of what is actually open,
# and the question is kept together with its answer — it explains the decision
# better than the decision alone.
#
# Modes:
#   archive-questions.sh           moves closed sections
#   archive-questions.sh --stage   the same, and stages both files if anything
#                                  moved — this is how pre-commit calls it: the
#                                  archive is usually not staged, and without
#                                  this the move would split into two commits
#
# The questions file is `open_questions_path` from `.rootstock.toml`; the
# archive is the same path with a `-closed` suffix
# (`open-questions.md` → `open-questions-closed.md`).
set -euo pipefail

root=$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)
answers="$root/.rootstock.toml"

mode=${1:-}
case "$mode" in
  "" | --stage) ;;
  --help | -h)
    sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    printf 'archive-questions.sh: unknown argument %s. Expected --stage or nothing.\n' "$mode" >&2
    exit 2
    ;;
esac

if [[ ! -f "$answers" ]]; then
  printf 'archive-questions.sh: no template answers file %s\n' "$answers" >&2
  exit 1
fi

# The `[answers]` table of the template record: `open_questions_path = "path"`.
open=$(awk -v q="'" '
  /^[ \t]*\[/ { inside = ($0 ~ /^[ \t]*\[answers\][ \t]*$/); next }
  inside && /^open_questions_path[ \t]*=/ {
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
if [[ -z "$open" ]]; then
  printf 'archive-questions.sh: %s has no key open_questions_path\n' "$answers" >&2
  exit 1
fi
open=${open#./}
closed="${open%.md}-closed.md"

cd "$root"
[[ -f "$open" ]] || exit 0

moved=$(perl -CSD -Mutf8 -0777 -e '
  my ($open, $closed) = @ARGV;
  open my $fh, "<:encoding(UTF-8)", $open or die "cannot read $open: $!\n";
  my $text = do { local $/; <$fh> };
  close $fh;

  # The document head is everything before the first section; then sections by "## ".
  my ($head, @rest) = split /^(?=## )/m, $text;
  my (@stay, @move);
  for my $s (@rest) {
    if ($s =~ /^## ~~/) { push @move, $s } else { push @stay, $s }
  }
  unless (@move) { print 0; exit 0 }

  # A removed section leaves its separator behind; collapse the doubles.
  my $body = $head . join("", @stay);
  $body =~ s/(?:^---\n\n){2,}/---\n\n/mg;
  open my $out, ">:encoding(UTF-8)", $open or die "cannot write $open: $!\n";
  print $out $body;
  close $out;

  my $archive = "";
  if (-e $closed) {
    open my $cf, "<:encoding(UTF-8)", $closed or die "cannot read $closed: $!\n";
    $archive = do { local $/; <$cf> };
    close $cf;
  } else {
    my $name = $open;
    $name =~ s{.*/}{};
    $archive = "# Closed questions\n\n"
      . "Questions that already have an answer. Moved here from `$name`\n"
      . "by `knowledge/archive-questions.sh` as soon as the heading is struck through.\n"
      . "Kept because a question together with its answer explains the decision\n"
      . "better than the decision alone.\n\n---\n\n";
  }
  $archive =~ s/\s*\z/\n\n/;
  open my $cf, ">:encoding(UTF-8)", $closed or die "cannot write $closed: $!\n";
  print $cf $archive, join("", @move);
  close $cf;
  print scalar @move;
' "$open" "$closed")

[[ "$moved" -gt 0 ]] || exit 0
printf 'closed questions moved: %d (%s → %s)\n' "$moved" "$open" "$closed"
if [[ "$mode" == --stage ]]; then
  git add -- "$open" "$closed"
fi
