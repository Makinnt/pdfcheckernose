# pdf-corrector

Corrector ortográfico de escritorio, **100% offline**. Abre PDFs, revisa ortografía en español e inglés en segundo plano, subraya errores sobre la página y permite omitirlos o inyectar notas nativas en el PDF.

Sin red, sin telemetría, sin dependencias C.

## Características

- Apertura de PDF con renderizado por página y navegación anterior/siguiente.
- Extracción de texto con coordenadas (origen abajo-izquierda, puntos PDF).
- Revisión ES/EN en segundo plano con barra de progreso y porcentaje.
- Subrayado rojo quemado en el bitmap + flash amarillo al seleccionar un error.
- Clasificación por tipo con color:
  - `Error` rojo — posible falta de ortografía.
  - `Mayúscula` azul — posible nombre propio.
  - `Sigla` naranja — posible acrónimo.
- Lista de errores con descripciones ES/EN, sugerencias bajo demanda, filtro `Todos | Error | Mayús. | Sigla`.
- Omitir / restaurar errores (se excluyen del subrayado).
- Notas adhesivas nativas (`Text annotation`) guardadas en `{nombre}_anotado.pdf` — jamás sobreescribe el original.
- Tema claro / oscuro propio (`PDF_DARK=1` para arrancar en oscuro).
- Apertura directa por argumento: `cargo run -- doc.pdf` (ideal para demos).

## Stack

- **Rust 2021**, binario único en `src/main.rs` (~780 líneas).
- UI declarativa con **Slint 1.10** en `ui/app.slint` (~290 líneas, controles propios, sin `std-widgets`).
- PDF con **PDFium** vía `pdfium-render` 0.8 (PDFium 157.0.8086.0, binarios en `assets/pdfium/`).
- Ortografía con **zspell 0.5** (`unstable-suggestions` solo por `suggest()`), Hunspell puro Rust.
- Dicts `es_ES` + `en_US` en `assets/dicts/` (fuente: `wooorm/dictionaries`).

## Requisitos

- Rust estable + Cargo.
- Linux: `libpdfium.so` ya incluido en `assets/pdfium/`, `build.rs` lo copia junto al exe. Alternativa: `PDFIUM_DYNAMIC_LIB_PATH=...`.
- Windows: se usa `assets/pdfium/pdfium.dll` del mismo modo.

## Uso rápido

```bash
cargo run -- /ruta/doc.pdf
# o sin argumento: elige el PDF en el diálogo

PDF_DARK=1 cargo run -- doc.pdf
cargo test --bin pdf-corrector
cargo build   # debug ~400MB por Slint, normal
```

Flujo típico: abre PDF → espera la barra de escaneo → clic en un error para ver sugerencias y saltar a su página → `Omitir` o `Añadir comentario` → la nota se guarda en `{nombre}_anotado.pdf`.

## Estructura

```
Cargo.toml      # zspell con features=["unstable-suggestions"]
build.rs        # compila ui/app.slint + copia libpdfium y dicts junto al exe
src/main.rs     # app completa: render, extracción, scan, subrayado, notas
ui/app.slint    # ventana, sidebar 400px, cards, filtros, temas
assets/pdfium/  # libpdfium.so + pdfium.dll + LICENSE.pdfium + licenses/ + VERSION.pdfium
assets/dicts/   # en_US.{aff,dic} + es_ES.{aff,dic}
input/          # PDFs de prueba
index.html      # prototipo web anterior, solo referencia de diseño/UX
```

En runtime los dicts y libpdfium se buscan en: `$PDFIUM_DYNAMIC_LIB_PATH` → junto al exe → `CARGO_MANIFEST_DIR/assets/...`. Los `.tgz` originales de Pdfium no se commitean.

## Cómo funciona

- `show()`: renderiza la página a 1400px, extrae palabras y pinta subrayados + flash. No toca el escaneo.
- `start_scan()` + `scan_tick()` (Timer 250ms, presupuesto 80ms): encola todas las páginas desde la visible, extrae + revisa por trozos en el hilo UI (no se crea un segundo `Pdfium` en otro hilo: se cuelga en `bind`).
- Solo 2 hilos fondo sin Pdfium: precarga de dicts al arrancar (`DictsReady`) y `suggest_for()` bajo demanda al clic.
- `State` central en `Rc<RefCell<State>>`: `all_errs` (todo el doc) + `view` (índices filtrados). Los handlers reciben índice de fila → mapear por `view`.
- `annotate()`: sticky-note nativa con `set_bounds` en el box del error.

## Límites conocidos

- Parseo `.dic/.aff` tarda ~30s en debug (~10x menos en release); se precarga mientras eliges archivo.
- `suggest()` es O(wordlist) por diseño de zspell: solo bajo demanda.
- Extracción = unión simple de boxes por carácter, sin shaping tipográfico.
- No hay reemplazo de texto en el PDF, solo sugerencias de referencia + notas.

## Licencia

MIT — ver `LICENSE`. PDFium: BSD + third-party en `assets/pdfium/licenses/`.
