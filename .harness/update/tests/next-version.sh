#!/usr/bin/env bash
# Тест next-version.sh: выбор последнего тега, пре-релизы, вывод describe,
# мажорная версия, --want и сравнение --lt.
set -uo pipefail

script="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)/next-version.sh"
pass=0
fail=0

ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  ПРОВАЛ %s\n     ожидалось: %s\n     получено: %s\n' "$1" "$2" "${3:-<пусто>}"; }

# Случай: имя, теги через пробел, аргументы скрипта, ожидаемый вывод одной
# строкой через пробел.
expect() {
  local name=$1 tags=$2 want=$3 got
  shift 3
  got=$(tr ' ' '\n' <<<"$tags" | "$script" "$@" 2>&1 | tr '\n' ' ')
  got=${got% }
  if [[ $got == "$want" ]]; then ok "$name"; else bad "$name" "$want" "$got"; fi
}

# Случай с ожидаемым кодом выхода 2.
expect_error() {
  local name=$1 tags=$2 code
  shift 2
  tr ' ' '\n' <<<"$tags" | "$script" "$@" >/dev/null 2>&1
  code=$?
  if [[ $code -eq 2 ]]; then ok "$name"; else bad "$name" "код 2" "код $code"; fi
}

# Сравнение: имя, a, b, ожидаемый код.
expect_lt() {
  local name=$1 code
  "$script" --lt "$2" "$3" >/dev/null 2>&1
  code=$?
  if [[ $code -eq $4 ]]; then ok "$name"; else bad "$name" "код $4" "код $code"; fi
}

tags="v0.1.0 v0.1.1 v0.2.0 v0.10.0 v0.9.3"

echo "== next-version.sh"
expect "последний тег по semver, а не по строке" "$tags" \
  "current=v0.1.0 next=v0.10.0 major=false" v0.1.0
expect "ссылки refs/tags/ и мусорные теги" "refs/tags/v0.1.0 refs/tags/v0.1.1 latest release-1" \
  "current=v0.1.0 next=v0.1.1 major=false" v0.1.0
expect "уже последняя — обновлять нечего" "$tags" \
  "current=v0.10.0 next= major=false" v0.10.0
expect "проект новее последнего тега — обновлять нечего" "$tags" \
  "current=v0.11.0 next= major=false" v0.11.0
expect "describe: коммиты после тега — версия тега" "$tags" \
  "current=v0.9.3 next=v0.10.0 major=false" v0.9.3-4-g4cf763c
expect "describe последнего тега — обновлять нечего" "$tags" \
  "current=v0.10.0 next= major=false" v0.10.0-1-gabc1234
expect "пре-релиз не берётся последним" "v0.1.0 v0.1.1 v1.0.0-rc.1" \
  "current=v0.1.0 next=v0.1.1 major=false" v0.1.0
expect "мажорная версия" "v0.1.0 v0.1.1 v1.0.0" \
  "current=v0.1.1 next=v1.0.0 major=true" v0.1.1
expect "мажор от 1.x" "v1.4.2 v2.0.0" \
  "current=v1.4.2 next=v2.0.0 major=true" v1.4.2
expect "minor — не мажор" "v1.4.2 v1.5.0" \
  "current=v1.4.2 next=v1.5.0 major=false" v1.4.2
expect "нет тегов — обновлять нечего" "" \
  "current=v0.1.0 next= major=false" v0.1.0
expect "--want: тег ниже последнего" "$tags" \
  "current=v0.1.0 next=v0.2.0 major=false" --want v0.2.0 v0.1.0
expect "--want: пре-релиз явно" "v0.1.0 v1.0.0-rc.1" \
  "current=v0.1.0 next=v1.0.0-rc.1 major=true" --want v1.0.0-rc.1 v0.1.0
expect "--want: не новее текущей — обновлять нечего" "$tags" \
  "current=v0.2.0 next= major=false" --want v0.1.1 v0.2.0
expect "с пре-релиза на релиз той же версии" "v1.0.0-rc.1 v1.0.0" \
  "current=v1.0.0-rc.1 next=v1.0.0 major=false" v1.0.0-rc.1
expect_error "--want: нет такого тега" "$tags" --want v0.3.0 v0.1.0
expect_error "текущая — не версия (хеш без тега)" "$tags" 4cf763c
expect_error "без текущей" "$tags"
expect_error "лишний аргумент" "$tags" v0.1.0 v0.1.1

expect_lt "--lt: patch" v0.1.0 v0.1.1 0
expect_lt "--lt: равные" v0.1.1 v0.1.1 1
expect_lt "--lt: больше" v0.2.0 v0.1.9 1
expect_lt "--lt: числа, а не строки" v0.9.0 v0.10.0 0
expect_lt "--lt: пре-релиз меньше релиза" v1.0.0-rc.1 v1.0.0 0
expect_lt "--lt: релиз не меньше пре-релиза" v1.0.0 v1.0.0-rc.1 1
expect_lt "--lt: rc.2 меньше rc.10" v1.0.0-rc.2 v1.0.0-rc.10 0
expect_lt "--lt: не версия — код 2" v1 v0.1.0 2

printf '\nnext-version.sh: %d ok, %d провалов\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
