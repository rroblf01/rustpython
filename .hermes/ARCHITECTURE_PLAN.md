# PLAN DE ARQUITECTURA Y RENDIMIENTO — RustPython + Django 6.0.7

> **Objetivo final**: RustPython ejecutando Django 6.0.7 **más rápido que CPython**.
> **Fecha**: 2026-07-14
> **Basado en**: Análisis del código fuente real (vm.rs, object.rs, jit.rs, compiler.rs, bytecode.rs, modules/)

---

## 1. ARQUITECTURA ACTUAL (MAPEO COMPLETO)

### 1.1 Pipeline de ejecución

```
source.py  ──▶  Lexer/Parser (parser.rs)  ──▶  AST (ast.rs)
                     │
                     ▼
             Compiler (compiler.rs)  ──▶  CodeObject (bytecode.rs)
                     │
                     ▼
        VirtualMachine.execute_inner()  (vm.rs)
                     │
                     ▼
       loop { execute_instruction() }  ──▶  match opcode
                     │
                     ▼
        Frame { stack, locals, globals, builtins }
```

### 1.2 Componentes clave

| Componente | Archivo | Líneas | Función |
|---|---|---|---|
| **Parser** | `src/parser.rs` | ~2043 | Recursive descent, produce AST |
| **Compiler** | `src/compiler.rs` | ~2780 | AST → CodeObject (bytecode) |
| **Bytecode** | `src/bytecode.rs` | ~318 | Opcode enum (CPython compat + REG_*) |
| **VM** | `src/vm.rs` | ~4168 | execute_instruction(), execute_inner(), call_function() |
| **Objects** | `src/object.rs` | ~8612 | PyObjectRef, PyObject (todos los tipos), PyDict, PySet |
| **JIT** | `src/jit.rs` | ~2010 | Cranelift JIT compiler (dormant) |
| **Módulos** | `src/modules/` (core.rs, dev.rs, misc.rs, text.rs, data.rs, files.rs, time.rs) | ~5000+ | ~50 módulos nativos |

### 1.3 Representación de objetos

```
PyObjectRef (16 bytes, repr(C)):
  ├── SmallInt(i64)         — inline, sin heap alloc
  ├── SmallBool(bool)       — inline
  ├── SmallFloat(f64)       — inline
  ├── SmallStr(SmallStr{data:[u8;15], len:u8}) — inline, <16 bytes
  ├── None
  ├── Mut(Rc<RefCell<PyObject>>)  — mutable heap: List, Dict, Set, Instance
  └── Imm(Rc<PyObject>)           — inmutable heap: Int, Str, Float, Tuple, Code, Function

PyObject (enum, heap):
  None, Bool, Int(BigInt), Float(f64), Str(String), Bytes, List, Tuple,
  Dict(PyDict), Set(PySet), Code(CodeObject), Function{...},
  BuiltinFunction{name, func: BuiltinFunc}, BuiltinMethod{...},
  Module{name, dict}, Type{name, dict, mro, ...}, Instance{dict, typ},
  BoundMethod{func, self_obj}, Property{getter, setter}, Cell{value},
  Generator, Coroutine, Slice, Range, RangeIter, ...
```

### 1.4 El JIT actual (dormante)

El archivo `src/jit.rs` implementa un compilador Cranelift completo:
- **~30 helper functions** `extern "C"` para aritmética, comparaciones, atributos, iteración, importación
- **`compile()`**: Toma un `CodeObject`, genera código nativo Cranelift con:
  - Fast path para SmallInt (tag checking + instrucciones nativas i64)
  - Fallback a helpers C para tipos complejos
  - Soporte de loops (JUMP_BACKWARD) y branching condicional
  - Soporte para ~30 opcodes (LOAD_FAST, LOAD_CONST, BINARY_OP, CALL, LOAD_ATTR, etc.)
- **Límite**: máximo 200 instrucciones, sin kwargs/*args
- **Estado**: `JitCompiler` se crea en `VirtualMachine::new()` pero **NUNCA se llama** — la función `run()` dice explícitamente "JIT compilation disabled — using stable interpreter path only"

---

## 2. ANÁLISIS DE CUELLOS DE BOTELLA

### 🔴 2.1 VM: Interpretación instrucción por instrucción

```
execute_inner():
  loop {
    match execute_instruction():
      Ok(None) => continue       // ← cada iteración paga costo de match + Option
      Ok(Some(val)) => return
      Err(e) => handle_exception
  }

execute_instruction():
  let fi = self.frames.len() - 1   // index check
  let ip = self.frames[fi].ip      // bounds check
  let op = instructions[ip].op     // bounds check
  let arg = instructions[ip].arg
  self.frames[fi].ip = ip + 1
  match op { ... }                  // indirect dispatch via enum match
```

**Problemas**:
- El `match op` gigante (~50+ branches) es un indirect branch — la CPU no puede predecirlo bien
- Cada instrucción paga overhead de: index check, bounds check, ip increment, profile check
- `SmallVec` push/pop no son gratuitos (shift de punteros)
- `cfg!(feature = "profile")` compila checks aunque no se usen (aunque el compilador las optimiza)

**Impacto**: **Crítico**. Cada instrucción Python ejecuta ~20-50 ops nativas solo en overhead de interpretación.

### 🔴 2.2 Atributos: MRO walk + sin cacheo efectivo

`LOAD_ATTR` (vm.rs línea 2104-2296):
- Para `Instance`: walk del dict de instancia, luego del dict del tipo, luego MRO completo
- ATTR_CACHE global: solo cachea `BuiltinFunction` por `(type_name, attr_name)` — es un HashMap thread-local con lock
- No cachea: resultados de property, classmethod, attribute lookups en general
- No cachea: resultados de `__getattr__` dinámicos

**Impacto**: **Muy alto**. Django hace millones de attribute lookups (modelos, settings, URL resolvers, template rendering).

### 🔴 2.3 Strings: clonado constante

- `py_str(&s)` → `PyObjectRef::Imm(Rc::new(PyObject::Str(s.to_string())))`
- Cada string literal se reconvierte en cada ejecución de `LOAD_CONST`
- `SmallStr` ayuda solo para <16 bytes
- Nombres de atributos se convierten a `String` en cada acceso

**Impacto**: **Alto**. Django procesa muchos strings (template tags, URL names, field names, etc.).

### 🔴 2.4 Frame allocation en cada llamada

```rust
let mut new_frame = Frame::new(
    Rc::new(code.clone()),  // clona el CodeObject
    func_globals.clone(),   // Rc clone
    ...
);
```

**Problemas**:
- Cada `call_function` aloca un nuevo `Frame` en el heap (dentro de `Vec<Frame>`)
- Cada Frame crea `attr_cache` y `global_cache` vecs del tamaño del code object
- `code.clone()` clona todo el CodeObject (instrucciones, consts, names)
- Frame se pushea al vec `self.frames` — puede causar reallocaciones

**Impacto**: **Alto**. Especialmente en hot loops.

### 🔴 2.5 Rc<RefCell<>> overhead sistemático

- Cada `borrow()` hace runtime borrow-checking (RefCell)
- Cada `borrow_mut()` puede panic en conflictos
- `Rc::clone()` incrementa/decrementa contador atómico
- Las inline variants (SmallInt, SmallBool, etc.) evitan Rc — pero las operaciones devuelven `PyObjectRef::Mut/Imm` que requiere Rc

**Impacto**: **Medio-Alto**. RefCell overhead es ~10-20ns por operación.

### 🔴 2.6 JIT completamente desconectado

- `JitCompiler` construido correctamente con Cranelift 0.109.1
- `compile()` implementado con soporte para loop/branch/30+ opcodes
- **Nunca se llama**. Ni en `run()`, ni en `call_function()`, ni en `execute()`
- Comentario en run(): "JIT compilation disabled — using stable interpreter path only"
- La función `jit_ptr: Cell<usize>` existe en `PyObject::Function` pero nunca se escribe

**Impacto**: **Oportunidad masiva**. Conectar JIT es el mayor win potencial.

### 🔴 2.7 Global cache sin invalidación

```rust
pub global_cache: Vec<Option<PyObjectRef>>,  // Frame field
```

- Se cachea LOAD_GLOBAL por instruction offset
- **Nunca se invalida** cuando cambian los globals
- Funciona porque es write-once (módulos normalmente no reasignan globals)
- Pero no soporta hot-reload ni exec/eval con nuevos globals

**Impacto**: **Medio**. Funciona para carga de módulos, pero no es robusto.

### 🔴 2.8 try_exec_simple: optimización tímida

```rust
fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
    if code.vararg_name.is_some() || ... { return None; }
    if instrs.is_empty() || instrs.len() > 12 { return None; }
```

- Solo funciones tiny (≤12 instrs)
- Sin varargs/kwargs
- No soporta LOAD_ATTR, CALL, BUILD_LIST complejo
- Es inline interpreter sin frame pero con el mismo match overhead

**Impacto**: **Bajo**. Ayuda en edge cases pero no en hot paths reales.

---

## 3. ARQUITECTURA PROPUESTA PARA RENDIMIENTO

### 3.1 Decisión estratégica: Bytecode optimizer + inline cache > JIT completo

**Recomendación**: No apostar todo al JIT en Fase 1. Un **bytecode optimizer + inline cache + register VM** puede dar 3-5x sin la complejidad de Cranelift.

**Razones**:
1. Cranelift ya está integrado pero no se usa porque compilar funciones pequeñas tiene overhead
2. Django tiene miles de funciones pequeñas → JIT cada una sería lento
3. El bottleneck real de Django no es aritmética (donde JIT ayuda), sino **attribute lookup, dict access, string handling**
4. Un bytecode optimizer (eliminación de dead code, plegado de constantes, inline de funciones pequeñas) beneficia TODO el código

**Estrategia híbrida**:
- **Fase 2 (corto plazo)**: Bytecode optimizer + inline caching agresivo + register VM
- **Fase 3 (medio plazo)**: JIT para hot loops detectados por profiling
- **Largo plazo**: JIT completo cuando la VM base ya sea rápida

### 3.2 Bytecode Optimizer (propuesta)

Crear `src/optimizer.rs` que transforme `CodeObject → CodeObject` optimizado:

1. **Constant folding**: `LOAD_CONST(1); LOAD_CONST(2); BINARY_OP(add)` → `LOAD_CONST(3)`
2. **Dead code elimination**: Instrucciones después de RETURN_VALUE
3. **Peephole**: `LOAD_CONST; RETURN_VALUE` en funciones triviales → inline directo
4. **Load/store elisión**: `LOAD_FAST(x); STORE_FAST(x)` → NOP
5. **Tuple/list unpack specialization**: `UNPACK_SEQUENCE(n)` en constantes conocidas
6. **Attribute pre-resolution**: Para módulos nativos, resolver LOAD_ATTR en compilación
7. **Register allocation**: Convertir stack bytecode a register bytecode (los REG_* opcodes ya existen)

### 3.3 Inline Cache avanzado

Sistema que cachea resultados de operaciones por instruction offset:

| Cache | Target | Estrategia |
|---|---|---|
| **LOAD_ATTR** | Instance.dict/Type.mro | Cachear offset directo en dict + type_version_tag para invalidación |
| **LOAD_GLOBAL** | globals/builtins | Ya existe pero necesita version_tag |
| **CALL** | BuiltinFunction | Cachear función pointer para evitar dispatch |
| **BINARY_OP** | SmallInt(smallint) | Tag check inline + operación nativa |
| **STORE_ATTR** | Instance.dict | Cachear slot offset si el tipo no cambia |

**Mecanismo de invalidación**: Cada `Type` tiene un `version_tag` que se incrementa cuando se modifica su dict. El inline cache almacena `(version_tag, offset/ptr)` y verifica antes de usar.

### 3.4 Register-based VM (ya existe parcialmente)

Los opcodes `REG_*` (0xC0-0xC9) ya están definidos y parcialmente implementados en `execute_instruction()`. La idea es:

- **Compilador produce bytecode stack-based** (CPython compatible)
- **Optimizer lo convierte a register-based** para hot functions
- La VM tiene 256 registros virtuales (`registers: Vec<Option<PyObjectRef>>`)
- Menos push/pop → menos bounds checks → menos memory traffic

### 3.5 JIT con Cranelift (reactivación)

Para reactivar el JIT existente:

1. **Profiling**: Activar `profile` feature para identificar hot functions
2. **Threshold**: Compilar solo funciones que se ejecutan > 1000 veces
3. **Compilación asíncrona**: No bloquear la VM mientras compila
4. **Inline del fast path de SmallInt**: El código ya lo hace para add/sub/mul — extender a más ops
5. **Eliminar límite de 200 instrucciones**: Usar compilación por bloques/trazas
6. **Soporte para kwargs/varargs**: Actualmente `compile()` retorna None para funciones con ellos

**Arquitectura JIT propuesta**:

```
Hot function detectada (profile counter > threshold)
    │
    ▼
Bytecode → Tracing JIT (solo hot path)
    │
    ▼
Cranelift compile() → función nativa
    │
    ▼
jit_ptr en Function actualizado
    │
    ▼
call_function() detecta jit_ptr != 0 y llama directamente
```

### 3.6 ¿Mantener compatibilidad CPython bytecode?

**SÍ, mantener bytecode CPython** como representación canónica.

Razones:
- Permite cargar `.pyc` files existentes (futuro)
- Facilita debugging (dis module, stack traces compatibles)
- El optimizer puede producir bytecode optimizado pero mantener el original para debugging

**Añadir**: Segundo bytecode register-based (los REG_* ya existen) como representación optimizada.

### 3.7 String interning

Implementar un `StringInterner` global:

```rust
// Propuesta
thread_local! {
    static STRING_INTERNER: RefCell<HashMap<&'static str, PyObjectRef>> = ...;
}
```

- Strings cortos comunes ("__init__", "self", "__class__", etc.) se internan
- `py_str()` revisa el interner primero
- Los strings de constantes literales se pre-resuelven en compilación
- Beneficio: menos allocs, comparaciones por puntero más baratas

---

## 4. CAPA DE COMPATIBILIDAD C

### 4.1 Análisis de necesidad para Django 6.0.7

Django 6.0.7 **no requiere C extensions** para funcionar básicamente. Las dependencias típicas:

| Dependencia | ¿C ext? | ¿Necesaria para Django minimal? |
|---|---|---|
| asgiref | No | Sí |
| sqlparse | No | Sí |
| pytz / zoneinfo | No | Sí |
| Python stdlib | Algunos módulos (_socket, _ssl, _hashlib, _json) | Sí |
| **numpy** | **Sí** | **No** (solo si usas Django + numpy) |
| **pandas** | **Sí** | **No** |
| **psycopg2** | **Sí** | **Sí para PostgreSQL** (pero psycopg3 tiene modo纯Python) |
| **mysqlclient** | **Sí** | **Solo para MySQL** |
| **uWSGI/gunicorn** | Parcial | No (usar daphne/uvicorn puro) |

**Conclusión**: Django puro puede ejecutarse **sin C extensions** si:
- Usas psycopg3 (pure Python mode) o SQLite
- Tienes stubs nativos para `_socket`, `_ssl`, `_hashlib`, `_json`, `_io`, `_imp`, `marshal`
- Usas servidor WSGI/ASGI puro Python (daphne, uvicorn)

### 4.2 Estrategia recomendada: "suficientemente compatible"

**No implementar Python/C API completa**. Enfoque por fases:

**Fase 1 (ahora)**: Stubs nativos Rust para módulos C del stdlib
- Estado actual: ~50 módulos nativos implementados (_imp, _io, _warnings, marshal, zipimport, socket, ssl, etc.)
- Lo que falta: _json, _elementtree, _pickle, _decimal, _csv, _multibytecodec
- **Acción**: Implementar stubs para los módulos C que Django necesita

**Fase 2 (futuro)**: Libffi/ctypes para cargar .so existentes
- Usar `libloading` (ya en Cargo.toml) + `libffi` para llamar funciones C
- Implementar subset de Python/C API: `PyArg_ParseTuple`, `PyObject_GetAttrString`, etc.
- Suficiente para cargar bibliotecas .so simples

**Fase 3 (no recomendada para este proyecto)**: Reimplementar numpy
- Reimplementar numpy en Rust sería un proyecto independiente de años
- Usar `ndarray` crate + wrappers Rust→Python
- **No intentar** para el objetivo Django

### 4.3 Python/C API subset necesario

Para cargar extensiones .so simples:

```c
// Funciones CPython ABI que habría que implementar
PyObject* PyModule_Create(PyModuleDef*);
int PyArg_ParseTuple(PyObject*, const char*, ...);
PyObject* PyObject_GetAttrString(PyObject*, const char*);
int PyObject_SetAttrString(PyObject*, const char*, PyObject*);
PyObject* PyLong_FromLong(long);
long PyLong_AsLong(PyObject*);
PyObject* PyUnicode_FromString(const char*);
// ... ~50 funciones más
```

**Implementación**: Wrapper Rust que:
1. Traduce punteros C `PyObject*` a `PyObjectRef` (via `Rc::into_raw` / `Rc::from_raw`)
2. Implementa ABI table con punteros a funciones
3. Usa `libloading` para cargar `.so` y resolver símbolos

---

## 5. MÉTRICAS Y BENCHMARKING

### 5.1 Baseline actual

Antes de optimizar, establecer mediciones. Script propuesto:

```python
# benchmark.py
import time
import django

# 1. Import time
t0 = time.time()
import django.conf  # Fresh import
t1 = time.time()
print(f"Import django.conf: {t1-t0:.3f}s")

# 2. URL resolve
from django.urls import resolve, reverse

# 3. Template render (simple)
from django.template import Template, Context
t = Template("Hello {{ name }}!")
for _ in range(1000):
    t.render(Context({"name": "World"}))
print(f"1000 template renders: {time.time()-t1:.3f}s")

# 4. ORM query (SQLite)
# ... etc
```

### 5.2 Métricas clave

| Métrica | CPython 3.13 | RustPython actual | Objetivo |
|---|---|---|---|
| Import django.conf | ~0.15s | ~2-5s? | <0.1s |
| 1000 template renders | ~0.02s | ? | <0.02s |
| Model creation (1000) | ~0.005s | ? | <0.005s |
| URL resolve (10000) | ~0.01s | ? | <0.01s |
| Bytes allocated | ~5MB | ? | <5MB |

### 5.3 Herramientas

- `cargo build --release && hyperfine './target/release/rustpython benchmark.py'`
- `valgrind --tool=callgrind` para identificar hot spots
- `perf stat` para cache misses, branch mispredictions
- Feature `profile` para conteo de instrucciones

---

## 6. ROADMAP DETALLADO

### Fase 0: Medición y estabilización (Semana 1)

**Objetivo**: Tener Django importando correctamente y métricas baseline.

- [ ] **Benchmark suite**: Script que mide tiempos de import, template render, URL resolve, ORM
- [ ] **Medir con CPython 3.13**: Baseline de referencia
- [ ] **Medir con RustPython actual**: Línea base
- [ ] **Corregir bugs de importación restantes**: Hacer que `import django` funcione sin errores
- [ ] **Stubs faltantes**: _json, _elementtree, _pickle (parcial), _decimal (parcial)
- [ ] **Identificar top 10 funciones más ejecutadas**: Usar feature `profile`

**Estimación**: 1 semana. Depende de cuántos bugs de import queden.

### Fase 1: Bytecode Optimizer + Inline Cache (Semanas 2-4)

**Objetivo**: 3-5x sobre baseline sin JIT.

#### 1.1 Bytecode optimizer (`src/optimizer.rs`) — Semana 2

- [ ] Implementar pase de constant folding
- [ ] Implementar pase de dead code elimination
- [ ] Implementar pase de peephole optimization
- [ ] Integrar en compiler pipeline: `compile()` → `optimize()` → CodeObject
- [ ] **Beneficio esperado**: ~20-30% menos instrucciones ejecutadas

#### 1.2 Inline cache avanzado — Semana 3

- [ ] Implementar `type_version_tag` en PyObject::Type
- [ ] Implementar `attr_cache` con version_tag en Frame (ya existe `attr_cache: Vec<Option<(u64, PyObjectRef)>>`)
- [ ] LOAD_ATTR inline: cachear offset directo en dict por tipo + version_tag
- [ ] LOAD_GLOBAL con version_tag (para invalidación correcta)
- [ ] **Beneficio esperado**: 2-5x en attribute-heavy code (template rendering, ORM)

#### 1.3 String interning — Semana 3-4

- [ ] Implementar `StringInterner` thread-local
- [ ] Pre-internar strings comunes ("__init__", "__class__", etc.)
- [ ] Pre-resolver strings de constantes literales en compilación
- [ ] **Beneficio esperado**: ~15% menos allocs, ~10% más rápido en string-heavy code

#### 1.4 Register VM optimization — Semana 4

- [ ] Convertir funciones hot de stack bytecode a register bytecode
- [ ] Eliminar push/pop overhead en hot paths
- [ ] **Beneficio esperado**: ~20% menos overhead de interpretación

**Hito Fase 1**: `import django.conf` en <0.5s, template rendering 3x faster que baseline.

### Fase 2: JIT Compiler (Semanas 5-8)

**Objetivo**: 10-15x sobre baseline para hot loops.

#### 2.1 Reactivar JIT path — Semana 5

- [ ] Conectar `jit_ptr` en `call_function()` para funciones compiladas
- [ ] Profiling automático con threshold (>1000 ejecuciones)
- [ ] Compilación asíncrona (background thread)
- [ ] **Esfuerzo**: Bajo. El código Cranelift ya está escrito.

#### 2.2 Mejorar cobertura de opcodes — Semana 5-6

- [ ] BUILD_MAP, BUILD_SET, BUILD_SLICE, BUILD_STRING
- [ ] STORE_SUBSCR, DELETE_SUBSCR
- [ ] LIST_APPEND, SET_ADD, MAP_ADD
- [ ] SETUP_WITH, WITH_EXIT (context managers son frecuentes en Django)
- [ ] LOAD_DEREF, STORE_DEREF (closures)
- [ ] MAKE_FUNCTION, CALL_FUNCTION_EX
- [ ] YIELD_VALUE, SEND (generators — Django los usa en template engine)
- [ ] **Esfuerzo**: Medio. Cada opcode requiere ~50-100 líneas de IR builder.

#### 2.3 Especialización de tipos — Semana 6-7

- [ ] **Monomorfización**: Si una función siempre recibe SmallInt, generar código nativo i64
- [ ] **Polymorphic inline cache (PIC)**: Para CALL con diferentes tipos
- [ ] **Guard checks**: Insertar type checks al inicio de bloques compilados
- [ ] **Deoptimización**: Si guard falla, saltar a interpreter

#### 2.4 Eliminar límite de 200 instrucciones — Semana 7

- [ ] Compilación por trazas (hot path dentro de la función, no toda la función)
- [ ] Loop unrolling para loops pequeños
- [ ] Inlining de funciones llamadas desde hot paths

#### 2.5 Soporte kwargs/varargs — Semana 8

- [ ] Implementar manejo de argumentos variables en código JIT
- [ ] Fast path para funciones sin kwargs (majoría de Django)
- [ ] **Esfuerzo**: Medio. Requiere cambios en calling convention.

**Hito Fase 2**: Hot loops Django (template engine, URL resolve) a velocidad nativa comparable a CPython.

### Fase 3: Optimizaciones avanzadas de VM (Semanas 9-10)

**Objetivo**: Superar CPython en todos los benchmarks.

#### 3.1 Frame pool / object pool

- [ ] Reutilizar frames en vez de allocar nuevos (pool de `Vec<Frame>`)
- [ ] Pool de `PyObjectRef` para tipos comunes (None, True, False, small ints)
- [ ] **Beneficio**: Eliminar malloc/free overhead en llamadas a función

#### 3.2 dict optimizado

- [ ] Reemplazar `PyDict` actual (HashMap<usize, Vec<(K,V)>>) con implementación más eficiente
- [ ] Usar `hashbrown::HashMap` directamente o implementar dict compacto estilo CPython
- [ ] Dict con key-sharing entre instancias de misma clase (__dict__)
- [ ] **Beneficio**: 2-3x en operaciones de dict (frecuentes en Django)

#### 3.3 Specialized calling conventions

- [ ] BuiltinFunction: evitar crear Vec de args si son pocos
- [ ] Fast path para `call_function` con 0-3 args sin heap alloc
- [ ] **Beneficio**: ~20% mejor en llamadas a función pequeñas

#### 3.4 String optimization

- [ ] SmallString inline (más grande: 31 bytes en vez de 15)
- [ ] CoW (Copy on Write) strings con Rc<str>
- [ ] **Beneficio**: ~10% mejor en string-heavy code

**Hito Fase 3**: RustPython ejecuta Django más rápido que CPython 3.13 en al menos 2 de 4 benchmarks.

### Fase 4: Soporte C ABI (Semanas 11-16)

**Objetivo**: Cargar extensiones C como psycopg2, numpy (básico).

#### 4.1 Python/C API subset — Semanas 11-13

- [ ] Implementar `PyObject` struct C-compatible que envuelva `PyObjectRef`
- [ ] Implementar ~50 funciones CPython ABI más comunes
- [ ] Tabla de symbols exportada para linked extensions
- [ ] **Esfuerzo**: Alto. ~2000+ líneas de wraers unsafe.

#### 4.2 ctypes/libffi — Semanas 13-14

- [ ] Implementar `ctypes` module (ya hay _ctypes en stdlib Python)
- [ ] Usar `libffi` crate para llamadas a funciones C
- [ ] **Esfuerzo**: Alto. ctypes es un módulo grande.

#### 4.3 Carga de .so existentes — Semana 15

- [ ] Usar `libloading` (ya en Cargo.toml) para cargar .so
- [ ] Enlazar Python/C API symbols
- [ ] Probar con extensiones simples (regex, xxhash, etc.)

#### 4.4 psycopg2 / uWSGI compatibility — Semana 16

- [ ] Testing con psycopg2 (si es necesario)
- [ ] Testing con uWSGI/gunicorn (si es necesario)
- [ ] Benchmark con Django real (PostgreSQL backend)

**Hito Fase 4**: RustPython puede cargar extensiones C del ecosistema Django.

---

## 7. RIESGOS Y MITIGACIONES

| Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|
| Cranelift 0.109.1 no soporta features modernos | Media | Alto | Evaluar upgrade a 0.110+ o switch a LLVM |
| Código JIT existente tiene bugs no detectados | Alta | Medio | Test suite con cobertura de opcodes |
| Stubs nativos incompletos rompen Django | Media | Alto | CI que corre tests de Django |
| RefCell conflicts en runtime | Baja | Alto | Sistema de borrow tracing en debug mode |
| Inline cache con version_tag memory explosion | Baja | Medio | Limitar tamaño de cache (LRU eviction) |
| Regresiones de rendimiento en CPython 3.14 | Baja | Medio | Benchmark comparativo continuo |
| No alcanzar velocidad CPython | Media | Alto | Priorizar Fase 2 (inline cache) sobre JIT |

---

## 8. DECISIONES TÉCNICAS CLAVE

### Decisión 1: ¿JIT parcial o completo?
**→ Parcial (tracing hot paths)**. Django tiene miles de funciones, compilar todas es caro. Mejor identificar hot paths via profiling.

### Decisión 2: ¿Bytecode propio o CPython?
**→ Ambos**. Mantener CPython bytecode como canónico. Generar bytecode register-based como optimización.

### Decisión 3: ¿Capa C ABI ahora o después?
**→ Después de Fases 1-3**. Django no necesita C extensions para funcionar. Priorizar velocidad de VM primero.

### Decisión 4: ¿Single-threaded o parallel?
**→ Single-threaded first**. Django request handling es single-threaded. Paralelismo (GIL removal) es un proyecto aparte.

### Decisión 5: ¿Mantener parser/compiler propio?
**→ Sí**. El parser/compiler RustPython ya funciona. Reescribirlo para usar CPython's compile sería más trabajo que beneficio.

---

## 9. PRÓXIMOS PASOS INMEDIATOS

1. **Ejecutar benchmark.py con CPython 3.13** para tener baseline
2. **Ejecutar benchmark.py con RustPython** (release build) para línea base
3. **Identificar top 10 bottlenecks** con profiling
4. **Comenzar Fase 0**: Stubs faltantes y estabilización de import
5. **Crear `src/optimizer.rs`** con constant folding + peephole
6. **Conectar JIT**: Eliminar comentario "JIT compilation disabled" en `run()`

---

## APÉNDICE A: RESUMEN DE ARCHIVOS REVISADOS

| Archivo | Líneas | Rol |
|---|---|---|
| `src/vm.rs` | 4168 | VM principal, execute_instruction, execute_inner, call_function, importación de módulos |
| `src/object.rs` | 8612 | Sistema de objetos completo: PyObjectRef, PyObject, PyDict, PySet, BuiltinFunc |
| `src/jit.rs` | 2010 | Compilador Cranelift con helpers extern "C" para 30+ opcodes |
| `src/compiler.rs` | 2780 | Compilador AST → CodeObject |
| `src/bytecode.rs` | 318 | Opcode enum + CodeObject + ConstValue |
| `src/parser.rs` | 2043 | Parser recursive descent |
| `src/ast.rs` | ~500 | Definiciones de AST |
| `src/modules/core.rs` | 1615 | builtins, math, sys, os, os.path, operator, etc. |
| `src/modules/dev.rs` | ~400 | _imp, _io, _warnings, marshal stubs |
| `src/modules/misc.rs` | ~1500 | re, threading, weakref, copy, types, itertools, datetime, etc. |
| `src/modules/data.rs` | ~300 | functools, collections |
| `src/modules/files.rs` | ~600 | glob, fnmatch, tempfile, shutil, gzip, tarfile, pathlib, zipfile |
| `src/modules/time.rs` | ~200 | time module |
| `src/modules/text.rs` | ~500 | string, textwrap, pprint, difflib, reprlib |
| `Cargo.toml` | 32 | Dependencies: cranelift 0.109.1, num-bigint, regex, libloading |
