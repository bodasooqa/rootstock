#!/usr/bin/env bash
# Тесты хуков обвязки и диспетчера. Каждый хук проверяется минимум двумя
# случаями: срабатывание и пропуск.
#
# Урок: хук, который может тихо ничего не сделать, хуже отсутствующего. Первая
# версия проверки типов молча получала пустую строку и завершалась с кодом 0,
# то есть делала вид, что проверка прошла. Поймано только намеренно сломанным
# типом.
#
# Всё гоняется на синтетических репозиториях во временных каталогах, а не на
# репозитории проекта: набор не зависит от его состояния. Случаи модульных
# хуков (rust-fmt, rust-clippy — модуль rust; js-fmt, js-typecheck — модуль
# frontend) идут, только если хук в проекте есть.
#
# Запуск: .harness/hooks/tests/run.sh
set -uo pipefail

# Под git-хуком (pre-push у lefthook) в окружении стоят GIT_DIR, GIT_INDEX_FILE
# и прочие: любая команда git тогда смотрит в репозиторий хука, а не в тот
# каталог, где её запустили. Хуки агента так не вызываются никогда, поэтому
# наследство снимается — иначе тесты проверяют не ту обстановку и «краснеют»
# ровно там, где всё в порядке.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
# Хуки берут корень проекта отсюда; тесты задают обстановку сами.
unset CLAUDE_PROJECT_DIR

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
hooks=$(dirname "$here")
root=$(cd "$hooks/../.." && pwd)

pass=0
fail=0
# Все временные каталоги — внутри одного, он убирается на выходе. Каталог
# внутри проекта (для biome) убирается отдельно.
base=$(mktemp -d)
jsdir=""

cleanup() {
  rm -rf "$base"
  [ -n "$jsdir" ] && rm -rf "$jsdir"
}
trap cleanup EXIT

ok()   { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad()  { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }
has()  { [ -e "$hooks/$1" ]; }

# Временный каталог, убираемый на выходе: печатает путь.
new_tmp() {
  mktemp -d "$base/tmp.XXXXXX"
}

# Временный git-репозиторий с пустым коммитом на main: печатает путь.
# Зовётся как `repo=$(new_repo) || exit 1`.
#
# Проверка отделения — не перестраховка. Один раз такой набор запустился из
# git-хука с унаследованным GIT_DIR: `git init` перевёл настоящий репозиторий
# в bare, `worktree add` завёл в нём ветку, а `commit` лёг на текущую ветку.
# Тест, способный писать в проверяемый репозиторий, опаснее непроверенного хука.
new_repo() {
  local d top want
  d=$(mktemp -d "$base/repo.XXXXXX")
  git -C "$d" init -q -b main
  top=$(git -C "$d" rev-parse --show-toplevel 2>/dev/null)
  want=$(cd "$d" && pwd -P)
  if [ "$top" != "$want" ]; then
    printf 'ОСТАНОВ: временный репозиторий не отделился (%s вместо %s). Набор не должен трогать рабочий репозиторий.\n' "${top:-<нет>}" "$want" >&2
    return 1
  fi
  git -C "$d" config user.email t@t
  git -C "$d" config user.name t
  git -C "$d" commit -q --allow-empty -m init
  printf '%s\n' "$d"
}

# check <имя> <скрипт> <json> <deny|context|silent> [рабочий каталог]
check() {
  local name=$1 script=$2 json=$3 expect=$4 dir=${5:-$root} out
  out=$(cd "$dir" && printf '%s' "$json" | "$hooks/$script" 2>/dev/null)
  case "$expect" in
    deny)
      case "$out" in *'"permissionDecision":"deny"'*) ok "$name" ;; *) bad "$name" "отказ" "$out" ;; esac ;;
    context)
      case "$out" in *'additionalContext'*) ok "$name" ;; *) bad "$name" "подсказка модели" "$out" ;; esac ;;
    silent)
      [ -z "$out" ] && ok "$name" || bad "$name" "молчание" "$out" ;;
    *) bad "$name" "известное ожидание" "$expect" ;;
  esac
}

bash_ev()   { jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}'; }
file_ev()   { jq -nc --arg f "$1" '{tool_name:"Edit",tool_input:{file_path:$f}}'; }
prompt_ev() { jq -nc --arg p "$1" '{hook_event_name:"UserPromptSubmit",prompt:$p}'; }

plain=$(new_tmp)

echo "guard-staging.sh"
staging_repo=$(new_repo) || exit 1
git -C "$staging_repo" worktree add -q "$staging_repo/wt" -b wt
check "git add -A в общем каталоге"     guard-staging.sh "$(bash_ev 'git add -A')"                     deny    "$staging_repo"
check "git add . в общем каталоге"      guard-staging.sh "$(bash_ev 'git add .')"                      deny    "$staging_repo"
check "git commit -am в общем каталоге" guard-staging.sh "$(bash_ev 'git commit -am "правка"')"        deny    "$staging_repo"
check "адресный git add"                guard-staging.sh "$(bash_ev 'git add src/lib.rs')"             silent  "$staging_repo"
check "-a внутри текста сообщения"      guard-staging.sh "$(bash_ev 'git commit -m "add -a to docs"')" silent  "$staging_repo"
check "git add -A в своём дереве"       guard-staging.sh "$(bash_ev 'git add -A')"                     silent  "$staging_repo/wt"
check "вне репозитория"                 guard-staging.sh "$(bash_ev 'git add -A')"                     silent  "$plain"

# Разбор heredoc ошибается только в сторону запрета: вырезанное хук не видит.
check "here-string не heredoc"          guard-staging.sh "$(bash_ev 'cat <<<x; git add -A')"           deny    "$staging_repo"
check "сдвиг в \$((…)) не heredoc"      guard-staging.sh \
  "$(bash_ev "$(printf 'echo $((1<<2)) x\ngit add -A\nx')")"                                         deny    "$staging_repo"
check "heredoc без терминатора"         guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<EOF >note.md\nтекст\nEOFX\ngit add -A')")"                            deny    "$staging_repo"
check "терминатор — слово без кавычек"  guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<E.O >note.md\nEO\ngit add -A\nE.O\ngit add note.md')")"               silent  "$staging_repo"
check "табы снимаются только при <<-"   guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<EOF >note.md\nтекст\n\tEOF\ngit add -A')")"                           deny    "$staging_repo"
check "git add -A в теле heredoc"       guard-staging.sh \
  "$(bash_ev "$(printf "cat <<'EOF' >note.md\ngit add -A\nEOF\ngit add note.md")")"                 silent  "$staging_repo"
check "git add -A в теле <<- с табом"   guard-staging.sh \
  "$(bash_ev "$(printf 'cat <<-"EOF" >note.md\n\tgit add -A\n\tEOF\ngit add note.md')")"            silent  "$staging_repo"
check "commit -F- с текстом в heredoc"  guard-staging.sh \
  "$(bash_ev "$(printf "git commit -F- <<'EOF'\nfix: убрал git add -A из инструкции\nEOF")")"         silent  "$staging_repo"
# Сомнение в разборе — вся команда на проверке. В каждом случае ниже настоящий
# bash либо прочёл бы строку как команду, либо хук не уверен, что нет.
for c in \
  'cat <<\\EOF >note.md\ngit add -A\nEOF' \
  "cat <<\$'EOF' >note.md\ngit add -A\nEOF" \
  'cat <<$"EOF" >note.md\ngit add -A\nEOF' \
  'cat <<E$X >note.md\ngit add -A\nE$X' \
  'echo "<<EOF"\ngit add -A\nEOF' \
  "echo '<<EOF'\ngit add -A\nEOF" \
  '(( x <<EOF ))\ngit add -A\nEOF' \
  'echo $((1 <<EOF))\ngit add -A\nEOF' \
  'echo $[1 <<EOF]\ngit add -A\nEOF' \
  'let x=1<<EOF\ngit add -A\nEOF' \
  'echo x # <<EOF\ngit add -A\nEOF' \
  "echo \$'x'; cat <<EOF >note.md\ngit add -A\nEOF" \
  'echo $"x"; cat <<EOF >note.md\ngit add -A\nEOF' \
  'echo $(( 1 + "2" )); cat <<EOF >note.md\ngit add -A\nEOF' \
  'echo $[ "1" ]; cat <<EOF >note.md\ngit add -A\nEOF' \
  'let "x = 1"; cat <<EOF >note.md\ngit add -A\nEOF' \
  'cat <<EOF >note.md \\\ngit add -A\nEOF'; do
  check "не heredoc: ${c%%\\n*}" guard-staging.sh "$(bash_ev "$(printf '%b' "$c")")" deny "$staging_repo"
done
check "команда после heredoc видна"     guard-staging.sh \
  "$(bash_ev "$(printf "cat <<'EOF' >note.md\nтекст\nEOF\ngit add -A")")"                           deny    "$staging_repo"

echo
echo "bash-edit.sh"
check "правка через sed -i"           bash-edit.sh "$(bash_ev "sed -i '' 's/a/b/' src/lib.rs")"  context
check "перенаправление в исходник"    bash-edit.sh "$(bash_ev 'cat > src/app.tsx')"              context
check "сборка"                        bash-edit.sh "$(bash_ev 'git status')"                     silent
check "запись во временный каталог"   bash-edit.sh "$(bash_ev 'echo done > /tmp/out.rs')"        silent
check "правка в теле heredoc"         bash-edit.sh \
  "$(bash_ev "$(printf 'cat <<%s\nsed -i x src/lib.rs\nEOF\n' "'EOF'")")"                         silent

echo
echo "session-tree.sh"
check "внутри репозитория"            session-tree.sh '{}'                                       context "$staging_repo"
check "вне репозитория"               session-tree.sh '{}'                                       silent  "$plain"

out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"ОБЩЕМ каталоге"*) ok "общий каталог назван" ;;
  *) bad "общий каталог назван" "строка про общий каталог" "$out" ;;
esac
out=$(cd "$staging_repo/wt" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"в своём дереве"*"Другие рабочие деревья"*) ok "своё дерево и соседи названы" ;;
  *) bad "своё дерево и соседи названы" "своё дерево и список соседей" "$out" ;;
esac

# Общий каталог сборки на все деревья: докладываем ровно тогда, когда он настроен.
out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"Общий каталог сборки настроен"*) bad "своего каталога сборки хватает" "молчание про target-dir" "$out" ;;
  *) ok "своего каталога сборки хватает" ;;
esac
mkdir -p "$staging_repo/.cargo"
printf '[build]\ntarget-dir = "/tmp/shared"\n' > "$staging_repo/.cargo/config.toml"
out=$(cd "$staging_repo" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
case "$out" in
  *"Общий каталог сборки настроен"*) ok "остаток общего каталога назван" ;;
  *) bad "остаток общего каталога назван" "жалоба на target-dir" "$out" ;;
esac
rm -rf "$staging_repo/.cargo"

# Git-хуки, установленные из дерева, ведут в его node_modules и умирают вместе
# с ним. Доклад — когда путь не в основном каталоге или файла нет; молчание —
# когда путь верен или репозиторий lefthook не использует.
main_phys=$(cd "$staging_repo" && pwd -P)
lefthook_hooks() {  # <путь к бинарю> — оба хука в форме, которую пишет lefthook 2
  local h
  for h in pre-commit pre-push; do
    printf '#!/bin/sh\ncall_lefthook()\n{\n  if test -n "$LEFTHOOK_BIN"\n  then\n    "$LEFTHOOK_BIN" "$@"\n  elif lefthook -h >/dev/null 2>&1\n  then\n    lefthook "$@"\n  elif %s -h >/dev/null 2>&1\n  then\n    %s "$@"\n  fi\n}\n' "$1" "$1" > "$staging_repo/.git/hooks/$h"
  done
}
hooks_report() {  # <имя> <broken|fine> [каталог]
  out=$(cd "${3:-$staging_repo}" && printf '{}' | "$hooks/session-tree.sh" 2>/dev/null)
  case "$2:$out" in
    broken:*"Git-хуки сломаны"*"lefthook install"*) ok "$1" ;;
    broken:*) bad "$1" "доклад о сломанных хуках" "$out" ;;
    fine:*"Git-хуки сломаны"*) bad "$1" "молчание про git-хуки" "$out" ;;
    fine:*) ok "$1" ;;
  esac
}
lefthook_hooks "$main_phys/wt/node_modules/lefthook/bin/lefthook"
hooks_report "без lefthook.yml про хуки молчит" fine
printf 'pre-commit: {}\n' > "$staging_repo/lefthook.yml"
# Бинарь есть и в дереве: путь туда неверен, даже пока дерево живо.
for d in "$staging_repo" "$staging_repo/wt"; do
  mkdir -p "$d/node_modules/lefthook/bin"
  printf '#!/bin/sh\n' > "$d/node_modules/lefthook/bin/lefthook"
  chmod +x "$d/node_modules/lefthook/bin/lefthook"
done
lefthook_hooks "$main_phys/node_modules/lefthook/bin/lefthook"
hooks_report "хуки ведут в основной каталог — молчит" fine
hooks_report "из дерева хуки в основном каталоге — молчит" fine "$staging_repo/wt"
lefthook_hooks "$main_phys/wt/node_modules/lefthook/bin/lefthook"
hooks_report "хуки ведут в дерево — доклад" broken "$staging_repo/wt"
lefthook_hooks "$main_phys/node_modules/gone/bin/lefthook"
hooks_report "путь в основной каталог, файла нет — доклад" broken
# lefthook, поставленный глобально: путь вне node_modules обязан существовать.
mkdir -p "$plain/global/bin"
printf '#!/bin/sh\n' > "$plain/global/bin/lefthook"
chmod +x "$plain/global/bin/lefthook"
lefthook_hooks "$plain/global/bin/lefthook"
hooks_report "глобальный lefthook на месте — молчит" fine
lefthook_hooks "$plain/gone/bin/lefthook"
hooks_report "глобальный lefthook пропал — доклад" broken
rm -f "$staging_repo/.git/hooks/pre-commit" "$staging_repo/.git/hooks/pre-push"
hooks_report "хуков нет — доклад" broken
rm -rf "$staging_repo/lefthook.yml" "$staging_repo/node_modules" "$staging_repo/wt/node_modules"

echo
echo "knowledge-due.sh"
# Хук ходит за отчётом в `knowledge/due.sh` своего проекта (два уровня вверх от
# каталога хуков). Проект синтетический: свои ответы, своя таблица этапов,
# вопрос к этапу в работе.
dueproj=$(new_tmp)
mkdir -p "$dueproj/.harness" "$dueproj/knowledge" "$dueproj/docs"
cp -R "$hooks" "$dueproj/.harness/hooks"
if [ -f "$root/knowledge/due.sh" ]; then
  cp "$root/knowledge/due.sh" "$dueproj/knowledge/"
  printf 'open_questions_path: knowledge/open-questions.md\nstages_path: docs/stages.md\n' > "$dueproj/.copier-answers.yml"
  printf '| Этап | Состояние |\n|---|---|\n| 1. Каркас | в работе |\n' > "$dueproj/docs/stages.md"
  printf '# Открытые вопросы\n\n## Вопрос\n\nСуть.\n\nНужно к: этапу 1.\n' > "$dueproj/knowledge/open-questions.md"
  due_check() {  # <имя> <prompt> <context|silent>
    local out
    out=$(printf '%s' "$(prompt_ev "$2")" | "$dueproj/.harness/hooks/knowledge-due.sh" 2>/dev/null)
    case "$3:$out" in
      context:*additionalContext*"Открытые вопросы к этапу 1"*) ok "$1" ;;
      context:*) bad "$1" "справка о вопросах этапа 1" "$out" ;;
      silent:) ok "$1" ;;
      silent:*) bad "$1" "молчание" "$out" ;;
    esac
  }
  due_check "перед созданием change"  '/opsx:propose новый экран' context
  due_check "обычное сообщение"       'почини тест'               silent
  due_check "apply того же change"    '/opsx:apply stage1-screen' silent
else
  bad "knowledge/due.sh на месте" "$root/knowledge/due.sh" "нет"
fi
# Справка, уронившая отправку сообщения, дороже отсутствующей справки:
# без knowledge/due.sh хук молчит с кодом 0.
rm -f "$dueproj/knowledge/due.sh"
out=$(printf '%s' "$(prompt_ev '/opsx:propose что-нибудь')" | "$dueproj/.harness/hooks/knowledge-due.sh" 2>&1); code=$?
{ [ $code -eq 0 ] && [ -z "$out" ]; } && ok "без knowledge/due.sh молчит" || bad "без knowledge/due.sh молчит" "молчание, код 0" "код $code: $out"

echo
echo "session-leftovers.sh"
tails=$(new_repo) || exit 1
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "чистое дерево — молчит" || bad "чистое дерево — молчит" "молчание" "$out"

printf 'черновик\n' > "$tails/draft.txt"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"незакоммиченных путей: 1"*) ok "называет незакоммиченное" ;;
  *) bad "называет незакоммиченное" "строка про незакоммиченные пути" "$out" ;;
esac

# `node_modules` симлинкуется в каждое дерево настройкой worktree и работой
# сессии не является: хук обязан его не считать.
rm "$tails/draft.txt"
mkdir "$tails/node_modules"
printf '{}\n' > "$tails/node_modules/package.json"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "node_modules не считается хвостом" || bad "node_modules не считается хвостом" "молчание" "$out"

# Своя ветка со своим коммитом и без upstream — её ещё никто не видел.
git -C "$tails" checkout -q -b feature
git -C "$tails" commit -q --allow-empty -m "своя работа"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"не отправлена ни разу"*) ok "называет неотправленную ветку" ;;
  *) bad "называет неотправленную ветку" "строка про неотправленную ветку" "$out" ;;
esac

out=$(cd "$plain" && "$hooks/session-leftovers.sh" 2>/dev/null)
[ -z "$out" ] && ok "вне репозитория — молчит" || bad "вне репозитория — молчит" "молчание" "$out"

# Расхождение с соседней веткой: два дерева правят один файл по-разному —
# два PR по append-only журналам.
git -C "$tails" checkout -q main
printf 'общая строка\n' > "$tails/journal.md"
git -C "$tails" add journal.md
git -C "$tails" commit -q -m "журнал"
git -C "$tails" worktree add -q "$tails/wt" -b neighbour
printf 'общая строка\nзапись соседа\n' > "$tails/wt/journal.md"
git -C "$tails/wt" add journal.md
git -C "$tails/wt" commit -q -m "запись соседа"

git -C "$tails" checkout -q -b mine
printf 'общая строка\nмоя запись\n' > "$tails/journal.md"
git -C "$tails" add journal.md
git -C "$tails" commit -q -m "моя запись"

out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"разойдётся с веткой neighbour"*journal.md*) ok "называет расхождение с соседней веткой" ;;
  *) bad "называет расхождение с соседней веткой" "строка про neighbour и journal.md" "$out" ;;
esac

# Тот же сосед, но правки в разных файлах — расхождения нет и выдумывать его
# нельзя: доклад, который жалуется всегда, читать перестанут.
git -C "$tails" checkout -q -B mine2 main
printf 'моё\n' > "$tails/only-mine.md"
git -C "$tails" add only-mine.md
git -C "$tails" commit -q -m "только свой файл"
out=$(cd "$tails" && "$hooks/session-leftovers.sh" 2>/dev/null)
case "$out" in
  *"разойдётся с веткой"*) bad "непересекающиеся правки расхождением не считаются" "молчание про neighbour" "$out" ;;
  *) ok "непересекающиеся правки расхождением не считаются" ;;
esac

fmtdir=$(new_tmp)
printf 'fn  main ( )  {\nlet x=1;\n}\n' > "$fmtdir/a.rs"
printf 'const  x  =1\n' > "$fmtdir/b.ts"

if has rust-fmt.sh; then
  echo
  echo "rust-fmt.sh"
  printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/rust-fmt.sh" >/dev/null 2>&1
  grep -q '^fn main() {$' "$fmtdir/a.rs" && ok "форматирует .rs" || bad "форматирует .rs" "rustfmt применён" "$(cat "$fmtdir/a.rs")"
  before=$(cat "$fmtdir/b.ts")
  printf '%s' "$(file_ev "$fmtdir/b.ts")" | "$hooks/rust-fmt.sh" >/dev/null 2>&1
  [ "$before" = "$(cat "$fmtdir/b.ts")" ] && ok "не трогает .ts" || bad "не трогает .ts" "файл без изменений" "$(cat "$fmtdir/b.ts")"
  printf 'fn  main ( )  {\nlet x=1;\n}\n' > "$fmtdir/a.rs"
fi

if has js-fmt.sh; then
  echo
  echo "js-fmt.sh"
  # Внутри проекта: biome берётся из его node_modules, правила — из его biome.json.
  jsdir="$root/.tmp-hook-test"
  mkdir -p "$jsdir"
  printf 'const  x  =   1;\nexport  {x}\n' > "$jsdir/a.ts"
  printf '%s' "$(file_ev "$jsdir/a.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  grep -q '^const x = 1;$' "$jsdir/a.ts" && ok "форматирует .ts" || bad "форматирует .ts" "biome применён" "$(cat "$jsdir/a.ts")"
  before=$(cat "$fmtdir/a.rs")
  printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  [ "$before" = "$(cat "$fmtdir/a.rs")" ] && ok "не трогает .rs" || bad "не трогает .rs" "файл без изменений" "$(cat "$fmtdir/a.rs")"
  # Фикстура — внешний ответ байт в байт: число в ней biome переписал бы.
  mkdir -p "$jsdir/crates/client/tests/fixtures"
  printf '{"amount":  114187000000.000000000000000000}\n' > "$jsdir/crates/client/tests/fixtures/oi.json"
  fix_before=$(cat "$jsdir/crates/client/tests/fixtures/oi.json")
  printf '%s' "$(file_ev "$jsdir/crates/client/tests/fixtures/oi.json")" | "$hooks/js-fmt.sh" >/dev/null 2>&1
  [ "$fix_before" = "$(cat "$jsdir/crates/client/tests/fixtures/oi.json")" ] \
    && ok "не трогает фикстуры" || bad "не трогает фикстуры" "файл без изменений" "$(cat "$jsdir/crates/client/tests/fixtures/oi.json")"
  # Файл соседнего дерева: исключения biome.json — от корня его дерева, а не
  # того, откуда запущен хук. Бинарь — только проектный: правленый файл может
  # лежать в чужом клоне, и его node_modules исполнять нельзя.
  if nb=$(new_repo); then
    mkdir -p "$nb/gen" "$nb/node_modules/.bin"
    printf '{"files":{"includes":["**","!gen/**"]}}\n' > "$nb/biome.json"
    printf '#!/bin/sh\ntouch "%s/ran"\n' "$nb" > "$nb/node_modules/.bin/biome"
    chmod +x "$nb/node_modules/.bin/biome"
    printf 'const  x  =   1;\n' > "$nb/gen/a.ts"
    printf 'const  x  =   1;\n' > "$nb/b.ts"
    (cd "$root" && printf '%s' "$(file_ev "$nb/gen/a.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1)
    grep -q '^const  x  =   1;$' "$nb/gen/a.ts" \
      && ok "соседнее дерево: исключение его biome.json" || bad "соседнее дерево: исключение его biome.json" "файл без изменений" "$(cat "$nb/gen/a.ts")"
    (cd "$root" && printf '%s' "$(file_ev "$nb/b.ts")" | "$hooks/js-fmt.sh" >/dev/null 2>&1)
    grep -q '^const x = 1;$' "$nb/b.ts" \
      && ok "соседнее дерево: форматирует" || bad "соседнее дерево: форматирует" "biome применён" "$(cat "$nb/b.ts")"
    [ ! -e "$nb/ran" ] \
      && ok "соседнее дерево: его biome не исполняется" || bad "соседнее дерево: его biome не исполняется" "проектный бинарь" "запущен $nb/node_modules/.bin/biome"
  else
    bad "соседнее дерево" "временный репозиторий" "не отделился"
  fi
fi

if has rust-clippy.sh; then
  echo
  echo "rust-clippy.sh — разбор пути и подпись ошибки против шквала"
  out=$(printf '%s' "$(file_ev "$fmtdir/b.ts")" | "$hooks/rust-clippy.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy пропускает не-.rs" || bad "clippy пропускает не-.rs" "молчание, код 0" "код $code: $out"
  out=$(printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/rust-clippy.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy пропускает файл вне крейта" || bad "clippy пропускает файл вне крейта" "молчание, код 0" "код $code: $out"

  # Правка-миграция трогает десятки файлов, и до последнего из них крейт красный
  # по одной причине. Хук обязан разбудить один раз, а не девяносто, — и обязан
  # разбудить снова, когда причина сменилась. Настоящий cargo здесь не нужен
  # и не годится: проверяется решение будить, а не сам линт. Проект годится
  # обеим раскладкам: крейт в crates/, и он же — корень с Cargo.toml.
  clippydir=$(new_repo) || exit 1
  clippystate=$(new_tmp)
  mkdir -p "$clippydir/crates/core/src" "$clippydir/bin"
  printf '[workspace]\n' > "$clippydir/Cargo.toml"
  printf 'fn main() {}\n' > "$clippydir/crates/core/src/lib.rs"
  fake="$clippydir/bin/clippy"
  printf '%s\n' '#!/usr/bin/env bash' 'cat "$FAKE_CLIPPY_OUT"; exit 1' > "$fake"
  chmod +x "$fake"
  printf 'error[E0061]: this method takes 3 arguments but 2 arguments were supplied\nerror: could not compile `demo-core` (test "a") due to 14 previous errors\n' \
    > "$clippystate/first.txt"
  # Тот же код ошибки, но другая формулировка, другой файл и другой счётчик —
  # ровно то, что даёт миграция по одному вызову. Подпись обязана совпасть.
  printf 'error[E0061]: this method takes 2 arguments but 1 argument was supplied\nerror: could not compile `demo-core` (test "b") due to 9 previous errors\n' \
    > "$clippystate/same.txt"
  printf 'error[E0599]: no method named `since_ms` found\nerror: could not compile `demo-core` (lib) due to 1 previous error\n' \
    > "$clippystate/other.txt"

  clippy_run() {
    CLAUDE_PROJECT_DIR="$clippydir" \
    FAKE_CLIPPY_OUT="$clippystate/$1.txt" \
    HARNESS_CLIPPY="$fake" \
    HARNESS_CLIPPY_STATE="$clippystate/state" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$clippydir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"
  }

  out=$(clippy_run first); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "первая ошибка будит" || bad "первая ошибка будит" "код 2 с текстом" "код $code: $out"
  out=$(clippy_run same); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "та же ошибка второй раз молчит" || bad "та же ошибка второй раз молчит" "молчание, код 0" "код $code: $out"
  out=$(clippy_run other); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "новая ошибка будит" || bad "новая ошибка будит" "код 2 с текстом" "код $code: $out"
  # Зелёный прогон забывает подпись: вернувшаяся ошибка обязана разбудить снова.
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$fake"
  clippy_run other >/dev/null 2>&1
  printf '%s\n' '#!/usr/bin/env bash' 'cat "$FAKE_CLIPPY_OUT"; exit 1' > "$fake"
  out=$(clippy_run other); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "после зелёного та же ошибка будит" || bad "после зелёного та же ошибка будит" "код 2 с текстом" "код $code: $out"

  # Исполнитель идёт по TDD: красное между его правками — рабочий ход, а не
  # сигнал. Признак субагента — поле `agent_id` события.
  agent_ev() { jq -nc --arg f "$1" '{tool_name:"Edit",tool_input:{file_path:$f},agent_id:"a1",agent_type:"developer"}'; }
  rm -rf "$clippystate/state"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" FAKE_CLIPPY_OUT="$clippystate/first.txt" HARNESS_CLIPPY="$fake" HARNESS_CLIPPY_STATE="$clippystate/state" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(agent_ev "$clippydir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "clippy молчит на правке субагента" || bad "clippy молчит на правке субагента" "молчание, код 0" "код $code: $out"
  out=$(clippy_run first); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } && ok "clippy будит на той же правке главной сессии" || bad "clippy будит на той же правке главной сессии" "код 2 с текстом" "код $code: $out"

  # Правка идёт пачкой: пока проверка считает, исполнитель трогает ещё файлы.
  # Вердикт о состоянии, которого на диске уже нет, будить не должен. Поддельный
  # линтер сам трогает соседний файл, пока «считает», — так окно гонки
  # воспроизводится точно, а не таймингом.
  staledir=$(new_repo) || exit 1
  stalestate=$(new_tmp)
  mkdir -p "$staledir/crates/core/src" "$staledir/bin"
  printf '[workspace]\n' > "$staledir/Cargo.toml"
  printf 'fn main() {}\n' > "$staledir/crates/core/src/lib.rs"
  printf 'pub fn a() {}\n' > "$staledir/crates/core/src/other.rs"
  stalefake="$staledir/bin/clippy"
  printf '%s\n' '#!/usr/bin/env bash' \
    'sleep 0.05' \
    '[ -n "${FAKE_TOUCH:-}" ] && touch "$FAKE_TOUCH"' \
    'printf "error[E0061]: сломано\n"' \
    'exit 1' > "$stalefake"
  chmod +x "$stalefake"
  sleep 1

  stale_run() {
    CLAUDE_PROJECT_DIR="$staledir" \
    FAKE_TOUCH=${1:-} \
    HARNESS_CLIPPY="$stalefake" \
    HARNESS_CLIPPY_STATE="$stalestate/state-$2" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$staledir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"
  }

  out=$(stale_run "$staledir/crates/core/src/other.rs" a); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "clippy молчит, когда дерево ушло вперёд" \
    || bad "clippy молчит, когда дерево ушло вперёд" "молчание, код 0" "код $code: $out"
  out=$(stale_run "" b); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy будит, когда вердикт свежий" \
    || bad "clippy будит, когда вердикт свежий" "код 2 с текстом" "код $code: $out"
  # Правка своего же файла устареванием не считается: `rust-fmt.sh` переписывает
  # ровно его и успел бы сделать это после метки.
  out=$(stale_run "$staledir/crates/core/src/lib.rs" c); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy будит, если тронут только сам правленый файл" \
    || bad "clippy будит, если тронут только сам правленый файл" "код 2 с текстом" "код $code: $out"

  # Файл чужого клона: cargo там исполнил бы его build.rs и макросы.
  rm -f "$staledir/ran"
  printf '%s\n' '#!/usr/bin/env bash' "touch '$staledir/ran'" 'exit 1' > "$stalefake"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-d" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$staledir/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy не запускается на файле чужого репозитория" \
    || bad "clippy не запускается на файле чужого репозитория" "молчание, код 0, без запуска" "код $code: $out$([ -e "$staledir/ran" ] && echo ' (запущен)')"
  # Подложный `.git`, указывающий на gitdir проекта, своим не делает: общий
  # каталог git у него проектный, а в списке деревьев проекта его нет.
  forged=$(new_tmp)
  mkdir -p "$forged/crates/core/src"
  printf '[workspace]\n' > "$forged/Cargo.toml"
  printf 'fn main() {}\n' > "$forged/crates/core/src/lib.rs"
  printf 'gitdir: %s/.git\n' "$clippydir" > "$forged/.git"
  rm -f "$staledir/ran"
  out=$(CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-e" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$forged/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy не запускается за подложным .git" \
    || bad "clippy не запускается за подложным .git" "молчание, код 0, без запуска" "код $code: $out$([ -e "$staledir/ran" ] && echo ' (запущен)')"
  # Соседнее дерево того же репозитория — своё.
  git -C "$clippydir" add -A && git -C "$clippydir" commit -qm crate
  git -C "$clippydir" worktree add -q "$clippydir.wt" 2>/dev/null
  out=$(CLAUDE_PROJECT_DIR="$clippydir" FAKE_CLIPPY_OUT="$clippystate/first.txt" HARNESS_CLIPPY="$fake" HARNESS_CLIPPY_STATE="$clippystate/state-wt" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$clippydir.wt/crates/core/src/lib.rs")" "$hooks/rust-clippy.sh"); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "clippy идёт по соседнему дереву того же репозитория" \
    || bad "clippy идёт по соседнему дереву того же репозитория" "код 2 с текстом" "код $code: $out"

  # Ответу git из чужого каталога верить нельзя: `core.worktree` в его конфиге
  # называет корнем дерево проекта, а запуск — в чужом каталоге.
  # shellcheck disable=SC2329 # зовётся через подстановку
  clippy_foreign() {  # <файл> <имя состояния>
    rm -f "$staledir/ran"
    CLAUDE_PROJECT_DIR="$clippydir" HARNESS_CLIPPY="$stalefake" HARNESS_CLIPPY_STATE="$stalestate/state-$2" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$1")" "$hooks/rust-clippy.sh"
  }
  if redirected=$(new_repo); then
    mkdir -p "$redirected/crates/core/src"
    printf '[workspace]\n' > "$redirected/Cargo.toml"
    printf 'fn main() {}\n' > "$redirected/crates/core/src/lib.rs"
    git -C "$redirected" config core.worktree "$clippydir"
    out=$(clippy_foreign "$redirected/crates/core/src/lib.rs" f); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
      && ok "clippy не верит core.worktree чужого клона" \
      || bad "clippy не верит core.worktree чужого клона" "молчание, код 0, без запуска" "код $code: $out$([ -e "$staledir/ran" ] && echo ' (запущен)')"
  fi
  # Клон внутри дерева проекта — тоже чужой: его корень по git — он сам.
  nested="$clippydir/vendor/other"
  mkdir -p "$nested/crates/core/src"
  git -C "$nested" init -q
  printf '[workspace]\n' > "$nested/Cargo.toml"
  printf 'fn main() {}\n' > "$nested/crates/core/src/lib.rs"
  out=$(clippy_foreign "$nested/crates/core/src/lib.rs" g); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy не запускается во вложенном клоне" \
    || bad "clippy не запускается во вложенном клоне" "молчание, код 0, без запуска" "код $code: $out$([ -e "$staledir/ran" ] && echo ' (запущен)')"
  # Вложенный клон, чей core.worktree называет корнем дерево проекта.
  git -C "$nested" config core.worktree "$clippydir"
  out=$(clippy_foreign "$nested/crates/core/src/lib.rs" h); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$staledir/ran" ]; } \
    && ok "clippy не верит core.worktree вложенного клона" \
    || bad "clippy не верит core.worktree вложенного клона" "молчание, код 0, без запуска" "код $code: $out$([ -e "$staledir/ran" ] && echo ' (запущен)')"
fi

if has js-typecheck.sh; then
  echo
  echo "js-typecheck.sh — разбор пути, субагент, устаревший вердикт"
  out=$(printf '%s' "$(file_ev "$fmtdir/a.rs")" | "$hooks/js-typecheck.sh" 2>&1); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc пропускает не-.ts" || bad "tsc пропускает не-.ts" "молчание, код 0" "код $code: $out"

  # Пакет годится обеим раскладкам: каталог packages/<имя> и он же — корень
  # своего git-репозитория с tsconfig.json. Компилятор — проекта, если он
  # поставлен: временный каталог без node_modules иначе тянул бы его из сети.
  tsc_bin=${HARNESS_TSC:-}
  [ -z "$tsc_bin" ] && [ -x "$root/node_modules/.bin/tsc" ] && tsc_bin="$root/node_modules/.bin/tsc"
  tsdir=$(new_tmp)
  for p in broken clean; do
    mkdir -p "$tsdir/packages/$p/src"
    git -C "$tsdir/packages/$p" init -q
    printf '{"compilerOptions":{"strict":true,"noEmit":true,"skipLibCheck":true,"types":[]},"include":["src"]}\n' > "$tsdir/packages/$p/tsconfig.json"
  done
  printf 'export const n: number = "строка";\n' > "$tsdir/packages/broken/src/index.ts"
  printf 'export const n: number = 1;\n' > "$tsdir/packages/clean/src/index.ts"
  # Каждый пакет — свой репозиторий, и проект для хука — пакет файла.
  ts_run() {  # <файл> [событие]
    CLAUDE_PROJECT_DIR="${1%/src/*}" HARNESS_TSC="$tsc_bin" bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "${2:-$(file_ev "$1")}" "$hooks/js-typecheck.sh"
  }
  if [ -n "$tsc_bin" ]; then
    out=$(ts_run "$tsdir/packages/broken/src/index.ts"); code=$?
    { [ $code -eq 2 ] && [ -n "$out" ]; } && ok "tsc ловит сломанный тип" || bad "tsc ловит сломанный тип" "код 2 с текстом ошибки" "код $code: $out"
    out=$(ts_run "$tsdir/packages/clean/src/index.ts"); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc молчит на здоровом пакете" || bad "tsc молчит на здоровом пакете" "молчание, код 0" "код $code: $out"
  else
    bad "компилятор TypeScript на месте" "$root/node_modules/.bin/tsc" "нет — нужен bun install"
  fi
  out=$(ts_run "$tsdir/packages/broken/src/index.ts" \
    "$(jq -nc --arg f "$tsdir/packages/broken/src/index.ts" '{tool_name:"Edit",tool_input:{file_path:$f},agent_id:"a1"}')"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } && ok "tsc молчит на правке субагента" || bad "tsc молчит на правке субагента" "молчание, код 0" "код $code: $out"

  # Подставная команда трогает соседний файл сама, и гонки в тесте не остаётся.
  stalets="$tsdir/packages/broken"
  printf 'export const m = 1;\n' > "$stalets/src/other.ts"
  stalefakets="$tsdir/tsc-fake"
  printf '%s\n' '#!/usr/bin/env bash' \
    'sleep 0.05' \
    '[ -n "${FAKE_TOUCH:-}" ] && touch "$FAKE_TOUCH"' \
    'printf "src/index.ts(1,14): error TS2322: сломано\n"' \
    'exit 1' > "$stalefakets"
  chmod +x "$stalefakets"
  sleep 1
  stale_ts_run() {
    CLAUDE_PROJECT_DIR="$stalets" FAKE_TOUCH=${1:-} HARNESS_TSC="$stalefakets" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ \
        "$(file_ev "$stalets/src/index.ts")" "$hooks/js-typecheck.sh"
  }
  out=$(stale_ts_run "$stalets/src/other.ts"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ]; } \
    && ok "tsc молчит, когда дерево ушло вперёд" \
    || bad "tsc молчит, когда дерево ушло вперёд" "молчание, код 0" "код $code: $out"
  out=$(stale_ts_run ""); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "tsc будит, когда вердикт свежий" \
    || bad "tsc будит, когда вердикт свежий" "код 2 с текстом" "код $code: $out"
  out=$(stale_ts_run "$stalets/src/index.ts"); code=$?
  { [ $code -eq 2 ] && [ -n "$out" ]; } \
    && ok "tsc будит, если тронут только сам правленый файл" \
    || bad "tsc будит, если тронут только сам правленый файл" "код 2 с текстом" "код $code: $out"

  # Файл чужого клона: tsc из его node_modules не исполняется.
  rm -f "$tsdir/ran"
  printf '%s\n' '#!/usr/bin/env bash' "touch '$tsdir/ran'" 'exit 1' > "$stalefakets"
  out=$(CLAUDE_PROJECT_DIR="$tsdir/packages/clean" HARNESS_TSC="$stalefakets" \
    bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$stalets/src/index.ts")" "$hooks/js-typecheck.sh"); code=$?
  { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$tsdir/ran" ]; } \
    && ok "tsc не запускается на файле чужого репозитория" \
    || bad "tsc не запускается на файле чужого репозитория" "молчание, код 0, без запуска" "код $code: $out$([ -e "$tsdir/ran" ] && echo ' (запущен)')"

  # Игнорируемое в дереве проекта — не проект: распакованный в tmp/ пакет
  # принёс бы свой tsc. Пакет и в корне — годится обеим раскладкам.
  if ign=$(new_repo); then
    printf 'tmp/\n' > "$ign/.gitignore"
    pkgdir="$ign/tmp/pkg/packages/x"
    mkdir -p "$pkgdir/src"
    for d in "$ign" "$pkgdir"; do
      printf '{"include":["src"]}\n' > "$d/tsconfig.json"
    done
    printf 'export const n = 1;\n' > "$pkgdir/src/index.ts"
    rm -f "$tsdir/ran"
    out=$(CLAUDE_PROJECT_DIR="$ign" HARNESS_TSC="$stalefakets" \
      bash -c 'printf "%s" "$1" | "$2" 2>&1' _ "$(file_ev "$pkgdir/src/index.ts")" "$hooks/js-typecheck.sh"); code=$?
    { [ $code -eq 0 ] && [ -z "$out" ] && [ ! -e "$tsdir/ran" ]; } \
      && ok "tsc не запускается в игнорируемом каталоге" \
      || bad "tsc не запускается в игнорируемом каталоге" "молчание, код 0, без запуска" "код $code: $out$([ -e "$tsdir/ran" ] && echo ' (запущен)')"
  fi
fi

echo
echo "dispatch.sh"
# Синтетическая обвязка: настоящий диспетчер, поддельные общие хуки слота,
# свои хуки проекта. Каждый хук пишет своё имя в журнал — так виден порядок.
dproj=$(new_tmp)
mkdir -p "$dproj/.harness/hooks" "$dproj/.harness/local/hooks"
cp "$hooks/dispatch.sh" "$dproj/.harness/hooks/"
dlog="$dproj/log"
fake_hook() {  # <путь> <имя для журнала> <тело> — тело получает событие на stdin
  mkdir -p "$(dirname "$1")"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" %s >> %s\n%s\n' "$2" "$dlog" "$3" > "$1"
  chmod +x "$1"
}
dispatch() {  # <слот> [событие] — печатает «код<TAB>stdout<TAB>stderr» одной строкой на поле
  local out err code
  : > "$dlog"
  out=$(printf '%s' "${2:-{\}}" | "$dproj/.harness/hooks/dispatch.sh" "$1" 2>"$dproj/err")
  code=$?
  err=$(cat "$dproj/err")
  d_code=$code d_out=$out d_err=$err
  d_log=$(tr '\n' ' ' < "$dlog")
}
local_dir="$dproj/.harness/local/hooks"

# Порядок: общие слота в своём порядке, затем свои по алфавиту.
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt ':'
fake_hook "$dproj/.harness/hooks/js-fmt.sh" js-fmt ':'
fake_hook "$local_dir/post-edit/20-b.sh" b ':'
fake_hook "$local_dir/post-edit/10-a.sh" a ':'
dispatch post-edit
[ "$d_log" = "rust-fmt js-fmt a b " ] && [ "$d_code" -eq 0 ] \
  && ok "порядок: общие, потом свои по алфавиту" || bad "порядок: общие, потом свои по алфавиту" "rust-fmt js-fmt a b" "код $d_code: $d_log"

# Хук получает событие целиком.
fake_hook "$local_dir/post-edit/10-a.sh" a 'jq -r .tool_input.file_path >> '"$dproj/seen"
dispatch post-edit "$(file_ev /x/y.rs)"
[ "$(cat "$dproj/seen" 2>/dev/null)" = "/x/y.rs" ] \
  && ok "хук получает событие на stdin" || bad "хук получает событие на stdin" "/x/y.rs" "$(cat "$dproj/seen" 2>/dev/null)"
fake_hook "$local_dir/post-edit/10-a.sh" a ':'

# Отсутствующий модульный хук пропускается.
rm "$dproj/.harness/hooks/js-fmt.sh"
dispatch post-edit
[ "$d_log" = "rust-fmt a b " ] \
  && ok "нет модульного хука — пропуск" || bad "нет модульного хука — пропуск" "rust-fmt a b" "$d_log"

# Код 2 останавливает слот и пробрасывает stderr.
fake_hook "$local_dir/post-edit/10-a.sh" a 'echo "крейт красный" >&2; exit 2'
dispatch post-edit
{ [ "$d_code" -eq 2 ] && [ "$d_err" = "крейт красный" ] && [ "$d_log" = "rust-fmt a " ]; } \
  && ok "код 2 останавливает и пробрасывает stderr" \
  || bad "код 2 останавливает и пробрасывает stderr" "код 2, «крейт красный», b не запущен" "код $d_code, stderr «$d_err», журнал $d_log"
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt 'echo "общий красный" >&2; exit 2'
dispatch post-edit
{ [ "$d_code" -eq 2 ] && [ "$d_log" = "rust-fmt " ]; } \
  && ok "код 2 общего хука не пускает свои" || bad "код 2 общего хука не пускает свои" "код 2, только rust-fmt" "код $d_code: $d_log"
fake_hook "$dproj/.harness/hooks/rust-fmt.sh" rust-fmt ':'
fake_hook "$local_dir/post-edit/10-a.sh" a ':'

# Пустой local: только общие.
fake_hook "$dproj/.harness/hooks/bash-edit.sh" bash-edit 'echo "текст"'
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ] && [ "$d_out" = "текст" ]; } \
  && ok "без своих хуков — только общие" || bad "без своих хуков — только общие" "код 0, bash-edit, «текст»" "код $d_code: $d_log / $d_out"
mkdir -p "$local_dir/post-bash"
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ]; } \
  && ok "пустой каталог своих хуков — только общие" || bad "пустой каталог своих хуков — только общие" "bash-edit" "$d_log"

# Свой хук без права на исполнение пропускается с предупреждением.
fake_hook "$local_dir/post-bash/off.sh" off ':'
chmod -x "$local_dir/post-bash/off.sh"
dispatch post-bash
{ [ "$d_code" -eq 0 ] && [ "$d_log" = "bash-edit " ] && [[ "$d_err" == *"off.sh не исполняемый"* ]]; } \
  && ok "неисполняемый свой хук пропущен с предупреждением" \
  || bad "неисполняемый свой хук пропущен с предупреждением" "только bash-edit, предупреждение" "код $d_code, журнал $d_log, stderr $d_err"
rm -f "$local_dir/post-bash/off.sh"

# Отказ общего и отказ своего: наружу один корректный JSON — первый отказ.
deny_body() { printf 'jq -nc --arg r %s '\''{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'\''' "$1"; }
fake_hook "$dproj/.harness/hooks/guard-staging.sh" guard-staging "$(deny_body 'общий-отказ')"
fake_hook "$local_dir/pre-bash/guard-data.sh" guard-data "$(deny_body 'свой-отказ')"
dispatch pre-bash "$(bash_ev 'git add -A')"
reason=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.permissionDecisionReason' 2>/dev/null)
{ [ "$d_code" -eq 0 ] && [ "$reason" = "общий-отказ" ] && [ "$d_log" = "guard-staging " ]; } \
  && ok "два отказа — один JSON, первый отказ, слот остановлен" \
  || bad "два отказа — один JSON, первый отказ, слот остановлен" "JSON с «общий-отказ», guard-data не запущен" "код $d_code, журнал $d_log, вывод $d_out"
fake_hook "$dproj/.harness/hooks/guard-staging.sh" guard-staging ':'
dispatch pre-bash "$(bash_ev 'git add -A')"
reason=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.permissionDecisionReason' 2>/dev/null)
{ [ "$reason" = "свой-отказ" ] && [ "$d_log" = "guard-staging guard-data " ]; } \
  && ok "отказ своего хука доходит" || bad "отказ своего хука доходит" "JSON с «свой-отказ»" "$d_out"

# Два additionalContext — один JSON с текстами через пустую строку.
ctx_body() { printf 'jq -nc --arg n %s '\''{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$n}}'\''' "$1"; }
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree "$(ctx_body 'первый')"
fake_hook "$local_dir/session-start/note.sh" note "$(ctx_body 'второй')"
dispatch session-start
ctx=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.additionalContext' 2>/dev/null)
event=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.hookEventName' 2>/dev/null)
{ [ "$ctx" = "$(printf 'первый\n\nвторой')" ] && [ "$event" = "SessionStart" ] && [ "$(printf '%s\n' "$d_out" | wc -l | tr -d ' ')" = 1 ]; } \
  && ok "два контекста склеены в один JSON" || bad "два контекста склеены в один JSON" "первый, пустая строка, второй" "$d_out"

# Простой текст рядом с JSON уходит в тот же контекст.
fake_hook "$local_dir/session-start/note.sh" note 'echo "простой текст"'
dispatch session-start
ctx=$(printf '%s' "$d_out" | jq -r '.hookSpecificOutput.additionalContext' 2>/dev/null)
[ "$ctx" = "$(printf 'первый\n\nпростой текст')" ] \
  && ok "простой текст рядом с JSON — в контекст" || bad "простой текст рядом с JSON — в контекст" "первый + простой текст" "$d_out"

# Без JSON простой текст печатается как есть.
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree 'echo "строка один"'
dispatch session-start
[ "$d_out" = "$(printf 'строка один\nпростой текст')" ] \
  && ok "только простой текст — как есть" || bad "только простой текст — как есть" "две строки" "$d_out"

# Ошибка хука — не отказ: остальные работают, код 1 без JSON.
fake_hook "$dproj/.harness/hooks/session-tree.sh" session-tree 'echo "сбой" >&2; exit 1'
dispatch session-start
{ [ "$d_code" -eq 1 ] && [ "$d_log" = "session-tree note " ] && [ "$d_out" = "простой текст" ] && [ "$d_err" = "сбой" ]; } \
  && ok "ошибка хука не останавливает слот" || bad "ошибка хука не останавливает слот" "код 1, оба хука, stderr «сбой»" "код $d_code, журнал $d_log, вывод $d_out, stderr $d_err"

dispatch no-such-slot
[ "$d_code" -eq 1 ] && [[ "$d_err" == *"неизвестный слот"* ]] \
  && ok "неизвестный слот — ошибка" || bad "неизвестный слот — ошибка" "код 1" "код $d_code: $d_err"

# Настоящая проводка: отказ валовой постановки через диспетчер проекта.
out=$(cd "$staging_repo" && printf '%s' "$(bash_ev 'git add -A')" | "$hooks/dispatch.sh" pre-bash 2>/dev/null)
case "$out" in
  *'"permissionDecision":"deny"'*) ok "dispatch pre-bash проекта отклоняет git add -A" ;;
  *) bad "dispatch pre-bash проекта отклоняет git add -A" "отказ" "$out" ;;
esac
out=$(cd "$staging_repo/wt" && printf '%s' "$(bash_ev 'git add -A')" | "$hooks/dispatch.sh" pre-bash 2>/dev/null)
[ -z "$out" ] && ok "dispatch pre-bash проекта пропускает git add -A в своём дереве" \
  || bad "dispatch pre-bash проекта пропускает git add -A в своём дереве" "молчание" "$out"

# --- lefthook ---------------------------------------------------------------
# Настоящие `lefthook.yml` и `.harness/lefthook.yml` проекта на временном
# репозитории. Бинарь — проекта (node_modules) либо из PATH.
lefthook_bin=""
if [ -x "$root/node_modules/.bin/lefthook" ]; then
  lefthook_bin="$root/node_modules/.bin/lefthook"
elif command -v lefthook >/dev/null 2>&1; then
  lefthook_bin=$(command -v lefthook)
fi

# Копия обвязки git-хуков и скриптов knowledge/ проекта во временный репозиторий.
lefthook_copy() {  # <каталог>
  mkdir -p "$1/.harness" "$1/knowledge"
  cp "$root/lefthook.yml" "$1/"
  cp "$root/.harness/lefthook.yml" "$1/.harness/"
  cp -R "$root/.harness/lefthook" "$1/.harness/"
  cp "$root/.copier-answers.yml" "$1/"
  cp "$root"/knowledge/*.sh "$1/knowledge/"
}

answer() {  # <ключ> — значение из файла ответов проекта
  awk -v key="$1" 'index($0, key ":") == 1 { v = substr($0, length(key) + 2); gsub(/^[ \t\047"]+|[ \t\047"]+$/, "", v); print v; exit }' \
    "$root/.copier-answers.yml"
}

echo
echo "pre-commit: архив закрытых вопросов"
# stage_fixed возвращает в индекс только staged-файлы, а архив закрытых вопросов
# скрипт пишет сам. Закрытие обязано уехать одним коммитом с архивом; коммит без
# закрытия архив не трогает.
if [ -z "$lefthook_bin" ]; then
  bad "lefthook на месте" "node_modules/.bin/lefthook или lefthook в PATH" "нет"
else
  kidir=$(new_repo) || exit 1
  lefthook_copy "$kidir"
  questions=$(answer open_questions_path)
  closed="${questions%.md}-closed.md"
  mkdir -p "$kidir/$(dirname "$questions")"
  printf '# Открытые вопросы\n\n---\n\n## Первый вопрос\n\nСуть.\n\n---\n\n## Второй вопрос\n\nСуть.\n' > "$kidir/$questions"
  git -C "$kidir" add -A
  git -C "$kidir" commit -q --no-verify -m base
  (cd "$kidir" && "$lefthook_bin" install >/dev/null 2>&1)

  # Коммит без закрытия: архив не появляется и в коммит не попадает.
  printf '\n---\n\n## Третий вопрос\n\nСуть.\n' >> "$kidir/$questions"
  git -C "$kidir" add "$questions"
  git -C "$kidir" commit -q -m question >/dev/null 2>&1
  out=$(git -C "$kidir" show --name-only --format= HEAD | sort | tr '\n' ' ')
  { [ "$out" = "$questions " ] && [ ! -e "$kidir/$closed" ]; } \
    && ok "коммит без закрытия архив не трогает" \
    || bad "коммит без закрытия архив не трогает" "$questions" "$out"

  # Закрытие: оба файла в одном коммите, хвоста в рабочем дереве нет.
  perl -i -pe 's/^## Первый вопрос$/## ~~Первый вопрос~~ — закрыт 08.10.2026/' "$kidir/$questions"
  git -C "$kidir" add "$questions"
  git -C "$kidir" commit -q -m close >/dev/null 2>&1
  out=$(git -C "$kidir" show --name-only --format= HEAD | sort | tr '\n' ' ')
  rest=$(git -C "$kidir" status --porcelain)
  want=$(printf '%s\n%s\n' "$closed" "$questions" | sort | tr '\n' ' ')
  { [ "$out" = "$want" ] && [ -z "$rest" ]; } \
    && ok "закрытие и архив — один коммит" \
    || bad "закрытие и архив — один коммит" "$want, чистое дерево" "$out / $rest"
fi

echo
echo "pre-push: файлы пуша"
# Ветка заведена с --no-track и отстала от main, в которую после ответвления
# влит файл knowledge/. Пуш одного документа не должен будить knowledge-tests,
# а пуш своего файла knowledge/ обязан. Здесь важно лишь, пропущен джоб или нет.
if [ -n "$lefthook_bin" ]; then
  ppdir=$(new_tmp)
  git init -q --bare "$ppdir/origin.git"
  git init -q -b main "$ppdir/w"
  pp=$ppdir/w
  git -C "$pp" config user.email t@t
  git -C "$pp" config user.name t
  lefthook_copy "$pp"
  echo a > "$pp/knowledge/a.md"
  git -C "$pp" add -A
  git -C "$pp" commit -q --no-verify -m base
  git -C "$pp" remote add origin "$ppdir/origin.git"
  git -C "$pp" push -q --no-verify origin main
  git -C "$pp" switch -q -c docs --no-track
  echo d > "$pp/d.md"
  git -C "$pp" add d.md
  git -C "$pp" commit -q --no-verify -m docs
  git -C "$pp" switch -q main
  echo b > "$pp/knowledge/b.md"
  git -C "$pp" add knowledge/b.md
  git -C "$pp" commit -q --no-verify -m knowledge
  git -C "$pp" push -q --no-verify origin main
  git -C "$pp" switch -q docs

  # Состояние на момент `git push` новой ветки: HEAD — docs, upstream нет.
  push_job() { (cd "$pp" && "$lefthook_bin" run pre-push --job knowledge-tests --no-tty --colors off </dev/null 2>&1); }

  out=$(push_job)
  [[ $out == *"knowledge-tests (skip)"* ]] \
    && ok "docs отставшей ветки knowledge-tests не будят" \
    || bad "docs отставшей ветки knowledge-tests не будят" "knowledge-tests (skip)" "$out"

  # Контроль: без своего списка lefthook будит knowledge-tests чужим knowledge/b.md
  # из main. Если перестанет, тест выше ничего не доказывает.
  grep -v '^  files: \./\.harness/lefthook/push-files\.sh$' "$root/.harness/lefthook.yml" > "$pp/.harness/lefthook.yml"
  out=$(push_job)
  [[ $out != *"knowledge-tests (skip)"* ]] \
    && ok "контроль: встроенный список будит knowledge-tests чужим knowledge/" \
    || bad "контроль: встроенный список будит knowledge-tests чужим knowledge/" "без пропуска" "$out"
  cp "$root/.harness/lefthook.yml" "$pp/.harness/"

  echo c > "$pp/knowledge/c.md"
  git -C "$pp" add knowledge/c.md
  git -C "$pp" commit -q --no-verify -m own
  out=$(push_job)
  [[ $out != *"knowledge-tests (skip)"* ]] \
    && ok "свой файл knowledge/ будит knowledge-tests" \
    || bad "свой файл knowledge/ будит knowledge-tests" "без пропуска" "$out"
fi

echo
printf 'итого: %d прошло, %d провалено\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
