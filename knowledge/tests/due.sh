#!/usr/bin/env bash
# Тесты `knowledge/due.sh`. Проверка, стоящая на воротах, обязана быть проверена
# сама: гейт, который молча ничего не находит, хуже отсутствующего.
#
# Каждый случай — на срабатывание и на пропуск: просрочка находится там, где она
# есть, и не выдумывается там, где её нет. Проект синтетический, во временном
# каталоге: свой файл ответов, своя таблица этапов, свой список вопросов.
#
# Запуск: knowledge/tests/due.sh
set -uo pipefail

here=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
script="$here/../due.sh"

pass=0
fail=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

ok()  { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }

# Отдельный проект на случай. Пути вопросов и таблицы этапов — аргументами,
# по умолчанию как в форме catalog; список вопросов — со stdin.
setup() {
  local questions=${1:-knowledge/open-questions.md} stages=${2:-docs/spec/03-stages.md}
  rm -rf "$tmp/repo"
  mkdir -p "$tmp/repo/knowledge" "$tmp/repo/openspec/changes/archive" \
    "$tmp/repo/$(dirname "$questions")" "$tmp/repo/$(dirname "$stages")"
  cp "$script" "$tmp/repo/knowledge/due.sh"
  printf '[answers]\nopen_questions_path = "%s"\nstages_path = "%s"\n' "$questions" "$stages" > "$tmp/repo/.rootstock.toml"
  cat > "$tmp/repo/$stages" <<'MD'
# Этапы

| Этап | Состояние |
|---|---|
| 1. Каркас | закрыт |
| 2. Хранилище | закрыт |
| 3. API | в работе |
| 4–8. Остальное | не начаты |
MD
  {
    printf '# Открытые вопросы\n\n'
    cat
  } > "$tmp/repo/$questions"
}

run() {
  local out code
  out=$("$tmp/repo/knowledge/due.sh" "$@" 2>&1)
  code=$?
  printf '%s\n%s' "$code" "$out"
}

# check <имя> <ожидаемый код|!0> <ожидаемая подстрока|-> [аргументы due.sh]
check() {
  local name=$1 want_code=$2 want_text=$3
  shift 3
  local res code out
  res=$(run "$@")
  code=${res%%$'\n'*}
  out=${res#*$'\n'}
  if [ "$want_code" = "!0" ] && [ "$code" != 0 ]; then
    :
  elif [ "$code" != "$want_code" ]; then
    bad "$name" "код $want_code" "код $code: $out"
    return
  fi
  if [ "$want_text" != "-" ] && [[ "$out" != *"$want_text"* ]]; then
    bad "$name" "в выводе «$want_text»" "$out"
    return
  fi
  ok "$name"
}

echo "due.sh --overdue"

setup <<'MD'
## Вопрос к закрытому этапу

Суть.

Нужно к: этапу 2, где формат хранения всё равно потребует решения.
MD
check "вопрос к закрытому этапу — просрочка" 1 "Вопрос к закрытому этапу" --overdue

setup <<'MD'
## Вопрос к этапу впереди

Суть.

Нужно к: этапу 4, где появляется второй клиент.
MD
check "вопрос к будущему этапу — не просрочка" 0 "Просроченных вопросов нет" --overdue

setup <<'MD'
## Вопрос к этапу в работе

Суть.

Нужно к: этапу 3, где появляется API.
MD
check "вопрос к текущему этапу — не просрочка" 0 "Просроченных вопросов нет" --overdue

setup <<'MD'
## Вопрос без срока

Суть есть, а строки «Нужно к» нет.
MD
check "вопрос без срока — просрочка" 1 "строки «Нужно к:» нет" --overdue

setup <<'MD'
## Вопрос с прозаическим сроком

Суть.

Нужно к: этапу, где пользователей станет больше одного.
MD
check "прозаический срок — просрочка" 1 "срок не назван этапом" --overdue

setup <<'MD'
## ~~Закрытый вопрос~~ — закрыт 10.09.2026

Суть.

Нужно к: этапу 1.
MD
check "зачёркнутый вопрос не считается" 0 "Просроченных вопросов нет" --overdue

setup <<'MD'
## Вопрос к заархивированному change

Суть.

Нужно к: `stage3-public-api`, где это и станет видно.
MD
mkdir -p "$tmp/repo/openspec/changes/archive/2026-09-10-stage3-public-api"
check "change заархивирован — просрочка" 1 "заархивирован" --overdue

setup <<'MD'
## Вопрос к живому change

Суть.

Нужно к: `stage3-public-api`, где это и станет видно.
MD
check "change не заархивирован — не просрочка" 0 "Просроченных вопросов нет" --overdue

# Номер этапа из второго предложения сроком не является: срок — первое.
setup <<'MD'
## Срок в одном предложении, номер этапа — в другом

Суть.

Нужно к: этапу, где записей станет больше тысячи. С этапа 2 вопрос снят.
MD
check "номер этапа из соседнего предложения не берётся" 1 "срок не назван этапом" --overdue

# Пример формата в блоке кода — не вопрос, даже с «Нужно к:» без номера.
setup <<'MD'
Формат:

```markdown
## Заголовок вопроса

Нужно к: этапу N.
```

---
MD
check "пример формата в блоке кода не считается" 0 "Просроченных вопросов нет" --overdue

echo
echo "due.sh — пути из файла ответов"

# Форма monolith: вопросы и таблица этапов лежат там, куда указывают ответы.
setup docs/questions.md README.md <<'MD'
## Вопрос к закрытому этапу

Суть.

Нужно к: этапу 1.
MD
check "путь вопросов из ответов в отчёте" 1 "docs/questions.md:" --overdue

setup <<'MD'
## Вопрос

Нужно к: этапу 3.
MD
printf 'stages_path = "docs/spec/03-stages.md"\n\n[answers]\nopen_questions_path = "knowledge/open-questions.md"\n' > "$tmp/repo/.rootstock.toml"
check "нет stages_path в ответах (вне [answers] не в счёт) — ошибка" 1 "нет ключа stages_path" --overdue

setup <<'MD'
## Вопрос

Нужно к: этапу 3.
MD
printf '# Этапы\n\nТаблицы нет.\n' > "$tmp/repo/docs/spec/03-stages.md"
check "нет таблицы этапов — ошибка" '!0' "не нашлось таблицы состояния этапов" --overdue

echo
echo "due.sh — отчёт"

setup <<'MD'
## Вопрос к этапу в работе

Суть.

Нужно к: этапу 3, где появляется API.
MD
check "без аргумента берётся этап в работе" 0 "Открытые вопросы к этапу 3"
check "номер этапа аргументом"             0 "Открытые вопросы к этапу 4" 4
check "к названному этапу вопросов нет"    0 "К этому этапу вопросов не адресовано" 4
check "непонятный аргумент"                2 "непонятный аргумент" --что-то

echo
printf 'итого: %d прошло, %d провалено\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
