#!/usr/bin/env bash
# Слот post-edit-async (PostToolUse/Write|Edit, асинхронный), модуль rust:
# линтует крейт, которому принадлежит файл. Выход 2 будит модель с текстом
# ошибки.
#
# Два предохранителя против шквала. Правка-миграция (смена подписи и перевод
# на неё десятков вызовов) трогает файлы по одному, и до последнего из них
# крейт красный по одной и той же причине. Без предохранителей каждая правка
# будила сессию тем же текстом — девяносто пробуждений подряд.
#
#   1. Замок на крейт: пока идёт предыдущий прогон, новый не запускается.
#      Он смотрел бы на то же дерево и сказал бы то же самое.
#   2. Подпись ошибки: будит только новая. Подпись — множество строк `error`
#      и `warning` без чисел, поэтому съехавшие номера строк и «due to N
#      previous errors» её не меняют, а НОВАЯ ошибка меняет и будит.
#
# Умолчать насовсем ни то, ни другое не может: clippy запускается заново на
# каждой правке, и первое же изменение причины (или её исчезновение) слышно.
#
# Третий предохранитель: устаревший вердикт не будит — см. `lib/staleness.sh`.
#
# Подмена для тестов: `HARNESS_CLIPPY` — команда вместо `cargo clippy`,
# `HARNESS_CLIPPY_STATE` — каталог состояния вместо каталога в git дерева.
set -uo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
# shellcheck source=lib/staleness.sh
. "$here/lib/staleness.sh"
# shellcheck source=lib/same-repo.sh
. "$here/lib/same-repo.sh"

input=$(cat)
# Правка субагента не будит главную сессию — см. `js-typecheck.sh`.
[ -n "$(printf '%s' "$input" | jq -r '.agent_id // empty' 2>/dev/null)" ] && exit 0
f=$(printf '%s' "$input" | jq -r '.tool_response.filePath // .tool_input.file_path // empty' 2>/dev/null) || exit 0
case "$f" in
  *.rs) ;;
  *) exit 0 ;;
esac

# Один крейт в корне: пакет — `rootstock`, область — корень проекта,
# которому принадлежит файл. Сборочный каталог — не исходник.
case "$f" in
  */target/* | target/*) exit 0 ;;
esac
case "$f" in
  /*) file_dir=${f%/*} ;;
  */*) file_dir=$PWD/${f%/*} ;;
  *) file_dir=$PWD ;;
esac
# git в каталоге файла — только когда он в дереве проекта (см. lib/same-repo.sh).
same_repo "$file_dir" || exit 0
crate_dir=$(git -C "$file_dir" rev-parse --show-toplevel 2>/dev/null) || exit 0
[ -f "$crate_dir/Cargo.toml" ] || exit 0
# git даёт физический путь (на macOS `/private/var/…` вместо `/var/…`); путь
# правленого файла приводится к тому же виду, иначе проверка устаревания
# не узнает в нём себя.
f="$(cd "$file_dir" && pwd -P)/${f##*/}"
pkg="rootstock"
# cargo исполняет build.rs, макросы и тулчейн из rust-toolchain.toml: файл
# чужого клона не повод запускать их.
same_repo "${crate_dir:-.}" || exit 0

# Состояние — своё на рабочее дерево: два дерева линтуют разные исходники,
# и подпись одного не должна гасить пробуждение в другом.
# Ключ — корень рабочего дерева, а не текущий каталог: правку может сделать
# субагент из любого подкаталога, и по `$PWD` состояние разъехалось бы
# на несколько копий, а с ним и весь смысл подписи.
# Место — каталог git этого дерева, а не общий временный: в `/tmp` каталог
# с предсказуемым именем заранее заведёт другой пользователь машины, и запись
# подписи пойдёт по его ссылке в чужой файл.
tree=${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || printf '%s' "$PWD")}
state=${HARNESS_CLIPPY_STATE:-$(git -C "$tree" rev-parse --path-format=absolute --git-path harness/clippy 2>/dev/null)}
[ -n "$state" ] || exit 0
mkdir -p "$state" 2>/dev/null || exit 0

lock="$state/$pkg.lock"
seen="$state/$pkg.seen"

# Замок старше десяти минут — след убитого прогона, а не работающего.
find "$lock" -maxdepth 0 -type d -mmin +10 -exec rmdir {} + 2>/dev/null

mkdir "$lock" 2>/dev/null || exit 0
mark=$(stale_marker)
trap 'rmdir "$lock" 2>/dev/null; stale_drop "$mark"' EXIT

clippy=${HARNESS_CLIPPY:-cargo clippy}
out=$(cd "$crate_dir" && $clippy -p "$pkg" --all-targets --all-features -q -- -D warnings 2>&1) && {
  # Зелено: прежняя подпись забывается, и та же ошибка, вернувшись, разбудит.
  rm -f "$seen"
  exit 0
}

# Дерево ушло вперёд, пока считали: правка пачкой продолжается, и вердикт уже
# не о том, что лежит на диске. Проверяется ДО подписи: записать подпись
# устаревшего вердикта значило бы заглушить им же следующее, свежее пробуждение
# с той же причиной.
if [ -n "$crate_dir" ] && stale_since "$mark" "$crate_dir" "$f"; then
  exit 0
fi

# Подпись: коды ошибок (E0061 и подобные) плюс текст ошибок без кода, без чисел.
# Одна и та же миграция даёт разные формулировки одного кода («takes 3 arguments
# but 2», «takes 2 arguments but 1») и разные наборы файлов — по коду они
# сходятся в одно. Строки «could not compile» выброшены: там пляшут имена
# тестов и счётчики, а причины в них нет.
signature=$(printf '%s' "$out" | awk '
  /could not compile/ { next }
  /^error\[/ { code = $0; sub(/^error\[/, "", code); sub(/\].*/, "", code); print code; next }
  /^(error|warning)/ { gsub(/[0-9]+/, ""); print; next }
' | sort -u)
[ -n "$signature" ] || signature=$out
signature=$(printf '%s' "$signature" | cksum | cut -d' ' -f1)

[ "$(cat "$seen" 2>/dev/null)" = "$signature" ] && exit 0
printf '%s' "$signature" > "$seen"

printf 'clippy: %s не проходит -D warnings\n\n%s\n' "$pkg" "$(printf '%s' "$out" | tail -60)" >&2
exit 2
