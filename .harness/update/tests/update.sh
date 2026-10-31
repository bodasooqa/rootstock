#!/usr/bin/env bash
# Тест update.sh на временных репозиториях: обновление проекта через rootstock.
# Вместо rootstock — заглушка, которая правит файлы и печатает конфликты так,
# как настоящий: тест проверяет склейку — коммит, вывод, коды выхода.
set -uo pipefail

# Под git-хуком в окружении стоят GIT_DIR и прочие: git смотрел бы не туда.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
export GIT_AUTHOR_NAME=bot GIT_AUTHOR_EMAIL=bot@example.invalid
export GIT_COMMITTER_NAME=bot GIT_COMMITTER_EMAIL=bot@example.invalid

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/update.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }
same() { if [[ $3 == "$2" ]]; then ok "$1"; else bad "$1" "$2" "$3"; fi; }

echo "== update.sh"

"$script" >/dev/null 2>&1
same "без тега — код 2" 2 "$?"
"$script" v1.0.0 v2.0.0 >/dev/null 2>&1
same "два тега — код 2" 2 "$?"

# Проект на rootstock v1.0.0.
new_project() {
  local dir="$tmp/$1"
  git init -q -b main "$dir"
  mkdir -p "$dir/.harness/update" "$dir/sub"
  cp "$script" "$dir/.harness/update/"
  printf 'schema = 1\nversion = "v1.0.0"\n' >"$dir/.rootstock.toml"
  printf 'a\n' >"$dir/a.txt"
  printf 'keep\n' >"$dir/sub/keep.txt"
  git -C "$dir" add -A && git -C "$dir" commit -qm init
  printf '%s' "$dir"
}

stubs="$tmp/stubs"
mkdir -p "$stubs"
log="$tmp/calls"

# rootstock: как настоящий — update требует чистое дерево и корень проекта;
# конфликты печатает `<вид> <путь>` и выходит с кодом 2. $RS_CODE — код
# выхода (по умолчанию 0, без конфликтов); при 1 файлы не трогает.
cat >"$stubs/rootstock" <<EOF
#!/usr/bin/env bash
echo "rootstock \$* @ \$(basename "\$PWD")" >>"$log"
[ -f .rootstock.toml ] || { echo "error: no record" >&2; exit 1; }
[ -z "\$(git status --porcelain)" ] || { echo "error: dirty" >&2; exit 1; }
code=\${RS_CODE:-0}
[ "\$code" = 1 ] && { echo "error: boom" >&2; exit 1; }
printf 'schema = 1\nversion = "v2.0.0"\n' >.rootstock.toml
echo a2 >a.txt
if [ "\$code" = 2 ]; then
  echo "markers a.txt"
  echo "deleted-by-project gone.txt"
  echo "markers a.txt"
fi
exit "\$code"
EOF
chmod +x "$stubs/rootstock"

# Чистое обновление: один коммит, конфликтов нет.
proj=$(new_project clean)
: >"$log"
out=$(cd "$proj" && ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 2>/dev/null)
same "чисто: код 0" 0 "$?"
same "чисто: конфликтов нет" "" "$out"
same "чисто: вызов из корня проекта" "rootstock update --to v2.0.0 --defaults --trust @ clean" "$(cat "$log")"
same "чисто: один коммит" "Обновление харнесса до v2.0.0" "$(git -C "$proj" log --format=%s main~1..main)"
same "чисто: в коммите запись и правка" "$(printf '.rootstock.toml\na.txt')" \
  "$(git -C "$proj" diff --name-only main~1 main)"
same "чисто: дерево чистое" "" "$(git -C "$proj" status --porcelain)"
same "чисто: подпись" "bot bot@example.invalid" "$(git -C "$proj" log -1 --format='%an %ae')"

# Конфликты (код 2): обновление закоммичено, конфликты — без повторов.
proj=$(new_project conflict)
: >"$log"
out=$(cd "$proj/sub" && RS_CODE=2 ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 2>/dev/null)
same "конфликт: код 0" 0 "$?"
same "конфликт: из подкаталога — rootstock в корне" "rootstock update --to v2.0.0 --defaults --trust @ conflict" "$(cat "$log")"
same "конфликт: список без повторов" "$(printf 'deleted-by-project gone.txt\nmarkers a.txt')" "$out"
same "конфликт: один коммит" "Обновление харнесса до v2.0.0" "$(git -C "$proj" log --format=%s main~1..main)"
same "конфликт: дерево чистое" "" "$(git -C "$proj" status --porcelain)"

# Сбой rootstock: код 1, коммита нет.
proj=$(new_project fail)
(cd "$proj" && RS_CODE=1 ROOTSTOCK="$stubs/rootstock" "$script" v2.0.0 >/dev/null 2>&1)
same "сбой: код 1" 1 "$?"
same "сбой: нет коммита обновления" 0 "$(git -C "$proj" log --format=%s | grep -c 'Обновление харнесса')"

printf '%d ok, %d провалов\n' "$pass" "$fail"
((fail == 0))
