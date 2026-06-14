# Rest (WebMagic)

**Rest** — это строго типизированный, компилируемый язык программирования системного уровня, спроектированный специально для создания высоконагруженных веб-сервисов. Язык транслируется напрямую в нативный машинный код через инфраструктуру LLVM, обеспечивая производительность на уровне C++ и беспрецедентную безопасность управления памятью.

```text
Pipeline:     Lexer → Parser → Sema → HIR → LLVM IR → Native Object
Memory Model: Deterministic ARC (Automatic Reference Counting)
Performance:  ~290,000 RPS (HTTP Server Benchmark)
```

---

## Архитектурные Принципы

Разработка языка **Rest** опирается на строгие инженерные принципы, направленные на минимизацию накладных расходов при максимальной предсказуемости выполнения.

### 1. Zero-Overhead Web (Нативный Веб-Фреймворк)
Сетевое взаимодействие интегрировано непосредственно в семантику языка. Декораторы маршрутов (такие как `@get` и `@post`) обрабатываются компилятором на этапе генерации кода (Codegen). Это позволяет исключить необходимость в промежуточных HTTP-библиотеках и роутерах. 
* Компилятор генерирует оптимальные C-совместимые функции-обертки (wrappers) для обработки запросов.
* Данные (HTTP Payload) передаются обработчикам через механизм Zero-Copy.

### 2. Детерминированное Управление Памятью (ARC)
Мы полностью отказались от использования сборщиков мусора (Stop-The-World GC), вносящих непредсказуемые задержки (latency) в работу высоконагруженных систем. 
* Память управляется автоматически с помощью алгоритма подсчета ссылок (Automatic Reference Counting).
* Структуры данных имеют скрытый 4-байтовый заголовок в куче (heap).
* Вызовы `__rest_retain` и `__rest_release` генерируются компилятором неявно.
* При достижении нулевого счетчика ссылок запускается алгоритм каскадной очистки графа объектов (Deep Free).

### 3. Системное Взаимодействие (FFI & Raw Pointers)
Для достижения полного контроля над железом (Zero-Dependency), Rest поддерживает прямое взаимодействие с ядром операционной системы и библиотеками C:
* **`extern fn`**: Прямая декларация и вызов системных функций (Syscalls).
* **Memory Control**: Нативная поддержка сырых указателей (`*T`, `*u8`), операторов взятия адреса (`&`) и разыменования (`*`).
* **Type Casting**: Строго контролируемое приведение типов (`as`) и вычисление размеров структур (`sizeof`) во время компиляции.

---

## Устройство Компилятора

Компилятор Rest (написан на Rust) представляет собой классический многопроходный (multi-pass) конвейер трансляции:

1. **Lexical & Syntax Analysis**: Строгий токенизатор и парсер методом рекурсивного спуска (Recursive Descent) для построения Абстрактного Синтаксического Дерева (AST).
2. **Semantic Analysis (TypeChecker)**: Строгая проверка типов, контроль корректности l-value выражений для работы с указателями и валидация FFI-контрактов.
3. **Lowering & HIR**: Понижение AST в высокоуровневое промежуточное представление (High-Level Intermediate Representation) для раскрытия синтаксического сахара.
4. **LLVM Codegen**: Модульная трансляция HIR в машинонезависимый LLVM IR с автоматическим внедрением алгоритмов ARC и HTTP-роутинга.

---

## Документация Проекта

Полная инженерная спецификация и техническая документация доступны в директории `docs/`:

- [Архитектура проекта и Workspace](docs/architecture.md)
- [Спецификация и Синтаксис языка](docs/language_reference.md)
- [Внутреннее устройство Компилятора](docs/compiler_internals.md)
- [Теория и реализация Веб-Фреймворка](docs/web_framework.md)
- [Глубокое Управление Памятью (ARC)](docs/memory_management.md)

---

## Сборка и Запуск

**Требования к окружению:**
* `Rust 1.85+`
* `LLVM 18+` (включая `llvm-config`)

Для гарантии чистоты среды сборки и совпадения версий LLVM предоставляется официальный `Dockerfile`.

```bash
# 1. Сборка контейнера с LLVM 18
docker build -t rest-compiler .

# 2. Компиляция компилятора Rest через Cargo
docker run --rm -v "$(pwd):/app" -w /app rest-compiler cargo build --release
```

---

## Лицензия

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
