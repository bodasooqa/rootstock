#!/usr/bin/env bash
# Тест update.sh на временных репозиториях: формат тега шаблона (copier или
# rootstock) и обновление проекта обоими инструментами. Вместо copier и
# rootstock — заглушки, которые правят файлы и печатают конфликты так, как
# настоящие: тест проверяет склейку — коммиты, порядок, коды выхода.
set -uo pipefail

# Под git-хуком в окружении стоят GIT_DIR и прочие: git смотрел бы не туда.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
export GIT_AUTHOR_NAME=bot GIT_AUTHOR_EMAIL=bot@example.invalid
export GIT_COMMITTER_NAME=bot GIT_COMMITTER_EMAIL=bot@example.invalid

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/update.sh"
markers_script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/conflict-markers.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }
same() { if [[ $3 == "$2" ]]; then ok "$1"; else bad "$1" "$2" "$3"; fi; }

lt=$(printf '<%.0s' 1 2 3 4 5 6 7)
eq=$(printf '=%.0s' 1 2 3 4 5 6 7)
gt=$(printf '>%.0s' 1 2 3 4 5 6 7)

echo "== update.sh"

# Шаблон: v1.0.0 и v1.1.0 на copier (v1.1.0 — аннотированный тег), v2.0.0
# с rootstock.toml.
tpl="$tmp/template"
git init -q -b main "$tpl"
echo '_subdirectory: template' >"$tpl/copier.yml"
git -C "$tpl" add -A && git -C "$tpl" commit -qm one && git -C "$tpl" tag v1.0.0
echo '# правка' >>"$tpl/copier.yml"
git -C "$tpl" commit -qam two && git -C "$tpl" tag -a -m v1.1.0 v1.1.0
git -C "$tpl" rm -q copier.yml
echo 'schema = 1' >"$tpl/rootstock.toml"
git -C "$tpl" add -A && git -C "$tpl" commit -qm three && git -C "$tpl" tag -a -m v2.0.0 v2.0.0
export TEMPLATE_URL="file://$tpl"

same "format: тег на copier" copier "$("$script" format v1.0.0 2>&1)"
same "format: аннотированный тег на copier" copier "$("$script" format v1.1.0 2>&1)"
same "format: тег с rootstock.toml" rootstock "$("$script" format v2.0.0 2>&1)"
"$script" format v9.9.9 >/dev/null 2>&1
same "format: тега нет — код 2" 2 "$?"
TEMPLATE_URL='' "$script" format v1.0.0 >/dev/null 2>&1
same "format: без TEMPLATE_URL — код 2" 2 "$?"
"$script" apply cargo v1.0.0 >/dev/null 2>&1
same "apply: неизвестный инструмент — код 2" 2 "$?"

# Проект на copier v1.0.0: с файлом ответов и скриптами обновления.
new_project() {
  local dir="$tmp/$1"
  git init -q -b main "$dir"
  mkdir -p "$dir/.harness/update"
  cp "$script" "$markers_script" "$dir/.harness/update/"
  printf '_commit: v1.0.0\n_src_path: %s\n' "$TEMPLATE_URL" >"$dir/.copier-answers.yml"
  printf 'a\n' >"$dir/a.txt"
  printf 'b\n' >"$dir/b.txt"
  git -C "$dir" add -A && git -C "$dir" commit -qm init
  printf '%s' "$dir"
}

stubs="$tmp/stubs"
mkdir -p "$stubs"
log="$tmp/calls"

# copier: правит a.txt, оставляет конфликт маркерами в b.txt.
cat >"$stubs/copier" <<EOF
#!/usr/bin/env bash
echo "copier \$*" >>"$log"
sed -i.bak 's/_commit: .*/_commit: v1.1.0/' .copier-answers.yml && rm -f .copier-answers.yml.bak
echo a2 >a.txt
printf '%s project\nb\n%s\nb2\n%s template\n' '$lt' '$eq' '$gt' >b.txt
EOF

# rootstock: как настоящий — update требует чистое дерево; импорт и обновление
# печатают конфликты `<вид> <путь>` и выходят с кодом 2. $RS_FAIL — подкоманда,
# которая падает с кодом 1.
cat >"$stubs/rootstock" <<EOF
#!/usr/bin/env bash
echo "rootstock \$*" >>"$log"
[ "\${RS_FAIL:-}" = "\$1" ] && { echo "error: boom" >&2; exit 1; }
case \$1 in
  import-copier)
    [ -z "\$(git status --porcelain)" ] || { echo "error: dirty" >&2; exit 1; }
    echo 'version = "v1.0.0"' >.rootstock.toml
    rm .copier-answers.yml
    echo "blocked docs"
    exit 2 ;;
  update)
    [ -z "\$(git status --porcelain)" ] || { echo "error: dirty" >&2; exit 1; }
    echo 'version = "v2.0.0"' >.rootstock.toml
    echo a2 >a.txt
    echo "blocked docs"
    echo "deleted-by-project gone.txt"
    exit 2 ;;
esac
exit 1
EOF
chmod +x "$stubs/copier" "$stubs/rootstock"

# Обновление copier: один коммит, конфликт — вид markers.
proj=$(new_project copier)
: >"$log"
out=$(cd "$proj" && COPIER="$stubs/copier" "$script" apply copier v1.1.0 2>/dev/null)
same "copier: код 0 и с конфликтом" 0 "$?"
same "copier: конфликты" "markers b.txt" "$out"
same "copier: аргументы" "copier update --trust --defaults --conflict inline --vcs-ref v1.1.0" "$(cat "$log")"
same "copier: один коммит" "Обновление харнесса до v1.1.0" "$(git -C "$proj" log --format=%s main~1..main)"
same "copier: дерево чистое" "" "$(git -C "$proj" status --porcelain)"
same "copier: подпись" "bot bot@example.invalid" "$(git -C "$proj" log -1 --format='%an %ae')"

# Перевод на rootstock: импорт отдельным коммитом, затем обновление.
proj=$(new_project rootstock)
: >"$log"
out=$(cd "$proj" && ROOTSTOCK="$stubs/rootstock" "$script" apply rootstock v2.0.0 2>/dev/null)
same "rootstock: код 0 и с конфликтами" 0 "$?"
same "rootstock: конфликты обоих шагов, без повторов" "$(printf 'blocked docs\ndeleted-by-project gone.txt')" "$out"
same "rootstock: вызовы" "$(printf 'rootstock import-copier\nrootstock update --to v2.0.0 --defaults --trust')" "$(cat "$log")"
same "rootstock: два коммита по порядку" "$(printf 'Обновление харнесса до v2.0.0\nПеревод харнесса на rootstock')" \
  "$(git -C "$proj" log --format=%s main~2..main)"
same "rootstock: импорт убрал ответы copier" "D	.copier-answers.yml" \
  "$(git -C "$proj" diff --name-status main~2 main~1 -- .copier-answers.yml)"
same "rootstock: дерево чистое" "" "$(git -C "$proj" status --porcelain)"

# rootstock падает — update.sh падает, следующего шага нет.
for step in import-copier update; do
  proj=$(new_project "fail-$step")
  : >"$log"
  (cd "$proj" && RS_FAIL=$step ROOTSTOCK="$stubs/rootstock" "$script" apply rootstock v2.0.0 >/dev/null 2>&1)
  same "rootstock: $step с кодом 1 — код 1" 1 "$?"
  same "rootstock: после сбоя $step нет коммита обновления" 0 \
    "$(git -C "$proj" log --format=%s | grep -c 'Обновление харнесса')"
  if [ "$step" = import-copier ]; then
    same "rootstock: после сбоя импорта update не зовётся" "rootstock import-copier" "$(cat "$log")"
  fi
done

printf '%d ok, %d провалов\n' "$pass" "$fail"
((fail == 0))
