# AGENTS.md — pdf-corrector

Corrector ortográfico de escritorio, **100% offline** (sin red ni telemetría).
Abre PDFs, extrae texto con coordenadas, revisa ortografía ES/EN en segundo plano,
subraya errores sobre la página y permite omitir + inyectar notas nativas.

Stack: **Rust 2021**, Slint (UI declarativa), `pdfium-render` 0.8 (PDFium),
`zspell` 0.5 + `unstable-suggestions` (Hunspell puro Rust, sin C).

## Estructura

```
Cargo.toml      # zspell lleva features=["unstable-suggestions"] (solo por suggest())
build.rs        # compila ui/app.slint + copia libpdfium y dicts junto al exe
src/main.rs     # TODO el código Rust (~780 líneas, un solo binario a propósito)
ui/app.slint    # TODO el UI (~290 líneas, controles propios, sin std-widgets)
assets/pdfium/  # libpdfium.so + pdfium.dll + LICENSE.pdfium + licenses/ + VERSION.pdfium
assets/dicts/   # en_US.{aff,dic} + es_ES.{aff,dic} (fuente: wooorm/dictionaries)
input/          # PDFs reales de prueba (El-viejo-y-el-mar.pdf)
index.html      # prototipo web anterior, solo referencia de diseño/UX
```

## Comandos

```bash
cargo build                      # debug (~400MB por Slint, normal)
cargo test --bin pdf-corrector   # 4 tests (coordenadas, subrayado, kinds, anotación real)
cargo run -- /ruta/doc.pdf       # argv[1] abre directo, sin diálogo (útil para demos)
PDF_DARK=1 cargo run -- doc.pdf  # arranca en tema oscuro
PDFIUM_DYNAMIC_LIB_PATH=...      # alternativa a tener la lib junto al exe
```

`build.rs` copia `assets/pdfium/{libpdfium.so,pdfium.dll}` (según SO) y
`assets/dicts/*` a `target/{debug,release}/` (exe) y `target/.../dicts/`.
En runtime se busca en: `$PDFIUM_DYNAMIC_LIB_PATH` → junto al exe →
`CARGO_MANIFEST_DIR/assets/...`. Los `.tgz` originales de Pdfium **no** se commitean.

## Arquitectura lógica (`src/main.rs`)

Estado central: `struct State` en `Rc<RefCell<State>>` (hilo UI).

- `pdfium, path, page, total` — documento actual (se **reabre por página**,
  evita líos de lifetimes `Pdfium/PdfDocument`; cachear si va lento).
- `words: Vec<Word>` — palabras de la página visible (`text + x0,y0,x1,y1` en
  puntos PDF, origen abajo-izq).
- `all_errs: Vec<Misspelling>` — hallazgos de TODO el doc
  (`word, sug, page, box, dismissed, kind`).
- `view: Vec<usize>` — índices a `all_errs` tras el filtro de tipo. Los handlers
  de UI reciben índice de **fila** → mapear por `view` antes de tocar `all_errs`.
- `scan_queue/scan_pos`, `scan_id`, `flash`, `dicts_ready`, `last_lang`, `tx`.

Flujos:

1. `show()` — renderiza la página (`PdfRenderConfig` 1400px) a `slint::Image`,
   extrae palabras y **quema subrayados rojos** + **wash amarillo** (flash) en el
   bitmap. No toca el escaneo (seguro llamarlo para refrescar).
2. `start_scan()` — encola todas las páginas (desde la visible). **No usa hilos.**
3. `scan_tick()` (Timer 250ms, presupuesto 80ms/tick) — extrae + `check_words`
   por página, actualiza `all_errs`, barra (`scan-progress`, `scan-text` con %).
   Espera a `dicts_ready`. Devuelve si tocó la página visible → `show()` re-subraya.
4. Hilos fondo (solo 2, sin Pdfium): precarga de dicts al arrancar (manda
   `DictsReady`) y `suggest_for()` bajo demanda al clic (escanea el wordlist
   entero — lento por diseño de zspell, nunca en hilo UI).
5. `SpellEvent = DictsReady | Sug(gen, idx, sug)`. `gen == scan_id` invalida
   respuestas viejas. Cambio de idioma se detecta en el Timer (`lang-idx !=
   last_lang` → `start_scan`).
6. `annotate()` — sticky-note nativa (`create_text_annotation` + `set_bounds`)
   en el box del error, guarda en `{nombre}_anotado.pdf`. **Jamás sobreescribe.**
7. Tipos: `kind_of()` → `Error` (rojo) | `Mayúscula` (azul, posible nombre propio)
   | `Sigla` (naranja, posible acrónimo). Colores de `index.html`.

## UI (`ui/app.slint`)

- Paleta propia `c-bg/c-fg/...` según `is-dark`. **Todo `Text` lleva color explícito**:
  el texto por defecto de Slint sigue el tema del SO (blanco en GNOME oscuro →
  invisible sobre sidebar blanco). Controles 100% propios (`MiniBtn`,
  `Rectangle+Text+TouchArea`); `std-widgets` se eliminó porque salía en blanco.
- **GOTCHA TouchArea**: (1) un `TouchArea` declarado después del contenido queda
  **encima y se traga los clics** (esto rompió Omitir/Nota una vez); los botones
  van en filas separadas sin cobertura exterior. (2) Todo `TouchArea` lleva
  `width/height: parent` explícito.
- `TextInput` con `accepted =>` guarda con Enter si hay texto. No existe
  `multi-line` en esta versión de Slint (solo una línea).
- `Flickable + VerticalLayout + for` para la lista (no `ListView` de std-widgets).
- Cards: franja de color por tipo + palabra + tipo/pág + descripción ES/EN +
  sugerencias (`No hay sugerencias`/`No suggestions` si vacío) + Omitir/Restaurar
  + Añadir comentario (editor inline) + filtro `Todos|Error|Mayús.|Sigla`.

## Techos conocidos (no son bugs)

- Parseo `.dic/.aff` tarda ~30s **en debug** (~10x menos en release); se precarga
  al arrancar mientras el usuario elige archivo.
- `suggest()` es O(wordlist) — solo bajo demanda, con `unstable-suggestions`.
- **No crear segundo `Pdfium` en otro hilo**: se cuelga en `bind`. Por eso el
  escaneo es troceado en el hilo UI.
- `OnceLock<Result<_, String>>` en vez de `get_or_try_init` (inestable en este toolchain).
- Extracción = unión simple de boxes por carácter, sin shaping tipográfico.
- No hay reemplazo de texto en el PDF (cirugía redact+reescribir, fuera del mínimo);
  las sugerencias son de referencia, como en `index.html`.

## Estado

Fase 1 (ventana+render) · Fase 2 (texto+coords) · Fase 3 (corrector fondo) ·
Fase 4 (subrayado, omitir, notas nativas) · Extras (tema claro/oscuro, progreso
con % izq→der verde, tipos con color, filtro, flash amarillo 5s, descripciones ES/EN).
Licencia proyecto: MIT. Pdfium: BSD + third-party en `assets/pdfium/licenses/`.
