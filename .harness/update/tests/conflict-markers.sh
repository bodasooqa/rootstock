#!/usr/bin/env bash
# Тест conflict-markers.sh на временном репозитории: настоящий конфликт (обычный
# и diff3) находится, похожие строки — нет. Маркеры в этом файле собираются
# printf'ом, а не пишутся с начала строки: иначе сам тест был бы конфликтом.
set -uo pipefail

# Под git-хуком в окружении стоят GIT_DIR и прочие: git смотрел бы не туда.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/conflict-markers.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }

lt=$(printf '<%.0s' 1 2 3 4 5 6 7)
eq=$(printf '=%.0s' 1 2 3 4 5 6 7)
gt=$(printf '>%.0s' 1 2 3 4 5 6 7)
bar=$(printf '|%.0s' 1 2 3 4 5 6 7)

# Новый репозиторий в $tmp/<имя>: печатает путь.
new_repo() {
  local dir="$tmp/$1"
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  printf '%s' "$dir"
}

# Случай: имя, каталог, ожидаемый код и ожидаемый вывод (файлы через пробел).
expect() {
  local name=$1 dir=$2 code_want=$3 out_want=$4 out code
  out=$(cd "$dir/sub" 2>/dev/null || cd "$dir" && "$script" 2>/dev/null)
  code=$?
  out=$(printf '%s' "$out" | tr '\n' ' ')
  out=${out% }
  if [[ $code -eq $code_want && $out == "$out_want" ]]; then
    ok "$name"
  else
    bad "$name" "код $code_want, «$out_want»" "код $code, «$out»"
  fi
}

echo "== conflict-markers.sh"

repo=$(new_repo clean)
printf 'a\nb\n' >"$repo/plain.txt"
expect "нет маркеров" "$repo" 0 ""

repo=$(new_repo conflict)
mkdir -p "$repo/sub"
printf 'name: x\n%s before updating\n  a: 1\n%s\n  a: 2\n%s after updating\n' "$lt" "$eq" "$gt" >"$repo/sub/ci.yml"
git -C "$repo" add sub/ci.yml
printf 'b\n%s ours\nx\n%s base\ny\n%s\nz\n%s theirs\n' "$lt" "$bar" "$eq" "$gt" >"$repo/diff3.md"
printf 'ok\n' >"$repo/fine.txt"
expect "конфликт: отслеживаемый и неотслеживаемый, diff3, из подкаталога" "$repo" 1 "diff3.md sub/ci.yml"

repo=$(new_repo lookalike)
printf 'Заголовок\n%s\n\n%s\n' "$eq" "$lt" >"$repo/heading.md"
printf '%s только начало\nтекст\n%s конец без середины\n' "$lt" "$gt" >"$repo/no-middle.txt"
printf '%s начало\n%s\nи не закрыт\n' "$lt" "$eq" >"$repo/no-end.txt"
printf 'echo "%s ours"\n  %s\n  %s theirs\n' "$lt" "$eq" "$gt" >"$repo/indented.sh"
printf '%s\n%s\n%s\n' "$gt x" "$eq" "$lt y" >"$repo/reversed.txt"
expect "похожие строки — не конфликт" "$repo" 0 ""

repo=$(new_repo ignored)
printf 'build/\n' >"$repo/.gitignore"
mkdir -p "$repo/build"
printf '%s a\n%s\n%s b\n' "$lt" "$eq" "$gt" >"$repo/build/out.txt"
printf '%s a\n%s\n%s b\n\0' "$lt" "$eq" "$gt" >"$repo/blob.bin"
expect "игнорируемые и бинарные пропускаются" "$repo" 0 ""

printf '\nconflict-markers.sh: %d ok, %d провалов\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
