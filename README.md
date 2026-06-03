# Ref — компилируемый язык с LLVM-бэкендом

**ref** — небольшой C-подобный язык, который компилируется напрямую в нативный код
через LLVM. Без виртуальной машины, без сборщика мусора, без рантайма. Прямой кодген
через `inkwell`-биндинги.

```
Tests:       119 passing
Pipeline:    Lexer → Parser → Sema → Codegen → LLVM IR → native
Backend:     LLVM 18+ via inkwell
Dependencies: 3 (inkwell, anyhow, clap)
```

---

## Возможности

- **C-подобный синтаксис** — `let`, `fn`, `struct`, `if/else`, `while`, `for`,
  `break/continue`, `return`
- **Статическая типизация** — `i8..i64`, `u8..u64`, `f32/f64`, `bool`, `string`,
  массивы `T[N]`, структуры, функции
- **Type inference** — типы выводятся из литералов и выражений; аннотации опциональны,
  но обязательны для целочисленных литералов без суффикса
- **Struct + manual ownership** — `Person{name: "Alice", age: 30}` → `malloc`; при
  выходе из scope → `free` рекурсивно по всем полям
- **String semantics** — `+` конкатенирует, `strdup` на `let`/assign/field,
  `__ref_strcat` для concat
- **Borrow checker** — `ref x` создаёт shared borrow; перемещение/переприсваивание
  заимствованной переменной запрещено до выхода borrow'а из scope
- **Use-after-move detection** — компилятор ловит перемещение переменной с
  последующим использованием, move через цепочку `let y = (x)`, перемещение
  заимствованной переменной в функцию
- **Copy semantics для примитивов** — `i32`, `bool`, `f32` и т.п. не помечаются
  как moved при `let y = x`; только owning-типы (string, struct, array)
- **Полный компилятор за пару подходов** — 4 этапа (lexer / parser / sema / codegen),
  ~6500 строк Rust
- **LLVM backend** — `inkwell` safe-биндинги, эмитит `.ll` / `.bc` / нативный бинарь
- **Нулевой рантайм** — единственная runtime-функция это `__ref_strdup` для строк

---

## Архитектура

ref — это классический компилятор в четыре прохода, каждый в отдельном модуле.

**Lexer** (`src/lexer/`) — ручной токенайзер байт за байтом. Поддерживает:
целые с суффиксами (`42u64`), hex/octal/binary литералы, float, строки с escape'ами,
все типы как keywords (`i32`, `bool`, `string`). Каждый токен несёт `Span` с позицией
для диагностик.

**Parser** (`src/parser/`) — recursive descent → AST с `Stmt` (Let, Fn, Struct, If,
While, For, Return, Break, Continue) и `Expr` (Literal, Ident, Binary, Unary, Call,
FieldAccess, ArrayIndex, Struct, ArrayLiteral, Ref, Assign). Каждый узел имеет `Span`.

**Sema** (`src/sema/`) — типизация + lower to HIR, разбита на два прохода:
- `typeck.rs` — инференс типов с `TypeContext` (иерархия scope'ов), диагностика
  несоответствий, range-check для целочисленных литералов по аннотации, проверка
  дубликатов функций/структур/полей, разрешение перегрузок
- `borrowck.rs` — линейный move/borrow tracking, `Type::is_copy()` для примитивов,
  `collect_borrow_sources` для `&x` / `&x.y` / `&x[i]`, use-after-move detection,
  move через цепочку, move-через-borrow rejection

**Codegen** (`src/codegen/`) — HIR → LLVM IR через `inkwell`:
- Struct → `malloc` нужного размера, GEP store полей; `free` рекурсивно проходит
  по вложенным struct/string полям
- String → `__ref_strdup` на `let`/assign/field, `__ref_strcat` для concat
- Borrow → копия указателя, без owner tracking
- Owner tracking — `Vec<Vec<Owner>>` per scope, на выходе из scope `free` всех
  оставшихся owners

```
        source
          │
          ▼
       Lexer  ──▶  Token + Span
          │
          ▼
      Parser  ──▶  AST
          │
          ▼
   ┌── Sema ──┐
   │ typeck   │  ──▶  HIR (типизированный)
   │ borrowck │
   └──────────┘
          │
          ▼
       Codegen  ──▶  LLVM IR  ──▶  .o / .bc / бинарник
```

### Структура модулей

| Слой        | Путь                    | Назначение                                                |
|-------------|-------------------------|-----------------------------------------------------------|
| Driver      | `src/driver.rs`         | Точка входа для `lib.rs`, оркестрирует compile pipeline   |
| CLI         | `src/main.rs`           | Парсинг аргументов (`clap`), диспатч на build/run/llvm    |
| Lexer       | `src/lexer/`            | Байт-уровневый токенайзер, `Token`, `TokenKind`, `Span`   |
| AST         | `src/parser/ast.rs`     | Определения `Stmt` и `Expr` с `Span`                      |
| Parser      | `src/parser/parser.rs`  | Recursive descent, struct literal, `for i in 0..N`        |
| Type system | `src/sema/ty.rs`        | `Type` enum, `is_copy()`, преобразования                  |
| Typeck      | `src/sema/typeck.rs`    | Инференс, диагностики, range-check литералов              |
| Borrowck    | `src/sema/borrowck.rs`  | Move/borrow tracker, `HirExpr::Ident { name, ty, span }`  |
| HIR         | `src/ir/hir.rs`         | Типизированный IR с `HirExpr`, `HirStmt`, `BinOp`, `UnOp` |
| Lower       | `src/ir/lower.rs`       | AST → HIR, резолвинг типов, span propagation              |
| Codegen     | `src/codegen/codegen.rs`| HIR → LLVM IR через `inkwell`                             |

---

## Демо: Conway's Game of Life

Полная реализация в `examples/game_of_life.rf` — 20×10 сетка, 15 поколений, три
классических паттерна: глидер (движется по диагонали), блокер (мигает с периодом 2),
блок (still life). Один файл, ~120 строк, без внешних библиотек, чистая
целочисленная арифметика.

```
$ ref run examples/game_of_life.rf
Conway's Game of Life in ref
============================

Legend:  #  alive   .  dead

--- Generation ---
.#..................
..#.................
###.................
....................
........###.........
....................
....................
..............##....
..............##....
....................

--- Generation ---
....................
..#.................
.##.................
.#.......#..........
.........#..........
.........#..........
....................
..............##....
..............##....
....................
...
done.
```

---

## Почему ref?

### vs C

C — низкоуровневый, без структурного borrow checking'а. `int* p = &x; free(p); use(p);`
компилируется и падает в runtime. ref ловит use-after-move на этапе компиляции
и имеет структуры с рекурсивным `free` на выходе из scope — без ручного malloc/free.

### vs Rust

Rust — production-grade язык с lifetime-аннотациями, дженериками, &mut, async.
ref — это **учебный / портфолио-проект**: минимальная реализация тех же идей
(linear types, borrow checking, move semantics) в ~6500 строк. Никакого полиморфизма,
lifetime-вывода, async — только базовый ownership + shared borrow.

### vs Zig

Zig — системный язык с comptime, explicit allocation, прекрасной интеграцией с C.
ref проще: один тип файлов, один синтаксис, нет comptime, нет `Allocator`-параметров.
Структуры с автоматическим `free` — единственная фича для удобства.

### Что ref делает иначе

1. **Минимальная кодовая база** — весь компилятор влезает в одну голову за вечер
   чтения, ~3000 строк Rust
2. **Прямой кодген в LLVM** — без промежуточных VM, без tree-walking
3. **Borrow checker без &mut** — только shared borrow (`ref`); use-after-move ловится
   честно, не через runtime проверки
4. **Структуры с автоматическим destroy** — `let p = Point{x: 1};` создаёт объект
   в куче, `free` вызывается при выходе из scope
5. **Честный список ограничений** — нет модулей, нет stdlib, нет дженериков.
   Каждое ограничение явно задокументировано в `docs/pizdec.md`

---

## Использование

```rust
struct Point { x: i32, y: i32 }

fn distance_squared(p: Point) -> i32 {
    return p.x * p.x + p.y * p.y;
}

fn main() {
    let p = Point { x: 3, y: 4 };
    let r = ref p;
    print(r.x);
    print(r.y);
    print(distance_squared(p));
}
```

### CLI

```bash
ref run hello.rf                # скомпилировать и сразу запустить
ref build hello.rf -o hello     # в нативный бинарь
ref build hello.rf -O 2         # с оптимизацией (0|1|2|3)
ref llvm hello.rf -o out.ll     # эмитить LLVM IR для отладки
ref llvm hello.rf -o out.bc     # эмитить bitcode
```

### Как библиотека

```rust
use ref::driver;
use std::path::Path;

let output = Path::new("out.o");
driver::run(source_code, &output, opt_level)?;
```

---

## Подмножество языка

```ebnf
ty     ::= i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64
         | f32 | f64 | bool | string
         | ty[N] | struct_name

stmt   ::= let ident [: ty] [= expr] ;
        | fn ident (params) [-> ty] { stmt* }
        | struct ident { field: ty, ... }
        | if expr { stmt* } [else { stmt* }]
        | while expr { stmt* }
        | for ident in expr .. expr { stmt* }
        | break ; | continue ; | return [expr] ;
        | expr ;

expr   ::= int | float | string | true | false
        | ident | expr . ident | expr [ expr ]
        | expr + expr | expr - expr | expr * expr | expr / expr
        | expr == expr | expr != expr
        | expr < expr | expr <= expr | expr > expr | expr >= expr
        | expr && expr | expr || expr
        | ref expr
        | ident { field: expr, ... }
        | ty [N] { expr, ... }
        | ident ( expr, ... )
```

Целочисленные литералы требуют суффикс или аннотацию типа:
`let x: u64 = 9223372036854775807;` или `let y = 42i32;` или `let z = 42;` (по умолчанию `i32`).

---

## Тесты

```bash
$ cargo test
test result: ok. 119 passed; 0 failed
```

Покрытие: литералы всех типов, арифметика (включая integer overflow rejection),
control flow, рекурсия (включая взаимную), структуры (создание, копирование,
присваивание в поля), массивы (int и string), строки (конкатенация, экранирование,
ownership), borrow checker (use-after-move, borrow-of-borrow, borrow-persists-across-if,
move-into-borrowed-field), граничные случаи u64/i64, `let x = void_expr` rejection.

---

## Что не реализовано

Это MVP / учебный проект. В коде намеренно отсутствуют:

- **Модули** — один файл = одна программа
- **Стандартная библиотека** — `__ref_strdup` хардкод встроен в каждый модуль,
  завязка на libc
- **Мутабельные ссылки (`&mut`)** — только shared borrow
- **Lifetime-аннотации** — функция не может объявить, что возвращает заимствованное
  значение
- **Дженерики** — `T` как параметр типа отсутствует
- **Замыкания** — нет лямбд

Подробный список найденных багов и архитектурных решений — в [`docs/pizdec.md`](docs/pizdec.md).

---

## Требования к сборке

- Rust 1.85+
- LLVM 18+ (через `inkwell`)
- `cc` для линковки объектных файлов
- Linux (не тестировалось на других платформах)

```bash
cargo build --release
cargo test
cargo clippy
```

---

## License

MIT License

Copyright (c) 2026 haataru

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
