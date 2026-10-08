#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        OnceLock,
        mpsc::{Sender, channel},
    },
};

use anyhow::Context;
use pdfium_render::prelude::*;
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode, VecModel};

slint::include_modules!();

#[derive(Clone, Debug)]
struct Word {
    text: String,
    // coords PDF en puntos, origen abajo-izquierda (igual que Pdfium)
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

struct State {
    // ponytail: recargamos el doc por página para evitar líos de lifetimes Pdfium/PdfDocument; cachear si va lento
    pdfium: Pdfium,
    path: Option<PathBuf>,
    page: u32,
    total: u32,
    words: Vec<Word>,
    page_w: f32,
    page_h: f32,
    img_w: u32,
    img_h: u32,
    all_errs: Vec<Misspelling>, // hallazgos de todo el doc (Fase 3+4)
    view: Vec<usize>, // índices a all_errs tras aplicar el filtro de tipo
    flash: Option<(usize, std::time::Instant)>, // error resaltado en amarillo
    scan_id: u64,           // generación del escaneo vigente; invalida Sug viejos
    scan_queue: Vec<u32>,   // páginas pendientes (orden desde la visible)
    scan_pos: usize,
    dicts_ready: bool,
    tx: Sender<SpellMsg>,
    last_lang: i32,
}

#[derive(Clone, Debug)]
struct Misspelling {
    word: String,
    sug: Vec<String>,
    page: u32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dismissed: bool,
    kind: String, // Error | Mayúscula | Sigla (colores de index.html)
}

/// Error: palabra desconocida. Mayúscula: posible nombre propio. Sigla: posible acrónimo.
fn kind_of(w: &str) -> &'static str {
    if w.len() > 1 && w.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        "Sigla"
    } else if w.chars().next().is_some_and(|c| c.is_uppercase()) {
        "Mayúscula"
    } else {
        "Error"
    }
}

fn kind_bar(kind: &str) -> slint::Color {
    match kind {
        "Mayúscula" => slint::Color::from_rgb_u8(25, 118, 210),
        "Sigla" => slint::Color::from_rgb_u8(245, 124, 0),
        _ => slint::Color::from_rgb_u8(211, 47, 47),
    }
}

fn desc_for(kind: &str, en: bool) -> &'static str {
    match (kind, en) {
        ("Mayúscula", false) => "Mayúscula no reconocida (¿nombre propio?)",
        ("Mayúscula", true) => "Unrecognized capitalized word (proper noun?)",
        ("Sigla", false) => "Sigla no reconocida (¿acrónimo?)",
        ("Sigla", true) => "Unrecognized acronym",
        (_, false) => "Posible falta de ortografía",
        (_, true) => "Possible spelling mistake",
    }
}

enum SpellEvent {
    DictsReady,
    Sug(u64, usize, Vec<String>), // (gen, índice en all_errs, sugerencias)
}
type SpellMsg = SpellEvent;

static DICT_ES: OnceLock<Result<zspell::Dictionary, String>> = OnceLock::new();
static DICT_EN: OnceLock<Result<zspell::Dictionary, String>> = OnceLock::new();

fn dict_base() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("dicts");
            if p.is_dir() {
                return p;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/dicts")
}

fn load_dict(which: &str) -> anyhow::Result<zspell::Dictionary> {
    let base = dict_base();
    let aff = std::fs::read_to_string(base.join(format!("{which}.aff")))?;
    let dic = std::fs::read_to_string(base.join(format!("{which}.dic")))?;
    Ok(zspell::builder().config_str(&aff).dict_str(&dic).build()?)
}

fn dict_cached(which: &str) -> anyhow::Result<&'static zspell::Dictionary> {
    let lock = if which == "en_US" { &DICT_EN } else { &DICT_ES };
    lock.get_or_init(|| load_dict(which).map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("dict {which}: {e}"))
}

fn suggest_in(d: &zspell::Dictionary, w: &str) -> Vec<String> {
    d.entry(w).suggest().unwrap_or_default().iter().take(5).map(|s| s.to_string()).collect()
}

/// mode: 0 auto, 1 es, 2 en. Error en auto = falla en ambos.
/// Devuelve (índice en words, palabra). Rápido: solo check().
fn check_words(words: &[Word], mode: i32) -> Vec<(usize, String)> {
    let es = (mode != 2).then(|| dict_cached("es_ES").ok()).flatten();
    let en = (mode != 1).then(|| dict_cached("en_US").ok()).flatten();
    if es.is_none() && en.is_none() {
        return vec![];
    }
    let mut out = vec![];
    for (wi, w) in words.iter().enumerate() {
        if w.text.chars().any(|c| c.is_numeric()) {
            continue;
        }
        let ok_es = es.map(|d| d.check(&w.text)).unwrap_or(false);
        let ok_en = en.map(|d| d.check(&w.text)).unwrap_or(false);
        let ok = match mode {
            1 => ok_es,
            2 => ok_en,
            _ => ok_es || ok_en,
        };
        if !ok {
            out.push((wi, w.text.clone()));
        }
    }
    out
}

/// ponytail: suggest() escanea todo el wordlist; solo bajo demanda y en fondo
fn suggest_for(word: &str, mode: i32) -> Vec<String> {
    let es = (mode != 2).then(|| dict_cached("es_ES").ok()).flatten();
    let en = (mode != 1).then(|| dict_cached("en_US").ok()).flatten();
    let mut sug = vec![];
    if mode != 2 {
        if let Some(d) = es {
            sug.extend(suggest_in(d, word));
        }
    }
    if mode != 1 {
        if let Some(d) = en {
            for s in suggest_in(d, word) {
                if !sug.contains(&s) {
                    sug.push(s);
                }
            }
        }
    }
    sug.truncate(5);
    sug
}

fn push_errs(ui: &AppWindow, st: &Rc<RefCell<State>>) {
    let filter = ui.get_err_filter();
    let en = ui.get_lang_idx() == 2;
    let all = st.borrow().all_errs.clone();
    let mut view = vec![];
    let mut rows = vec![];
    for (gi, e) in all.iter().enumerate() {
        if filter == 1 && e.kind != "Error"
            || filter == 2 && e.kind != "Mayúscula"
            || filter == 3 && e.kind != "Sigla"
        {
            continue;
        }
        view.push(gi);
        rows.push(ErrRow {
            word: e.word.clone().into(),
            sug: if e.sug.is_empty() {
                (if en { "No suggestions" } else { "No hay sugerencias" }).into()
            } else {
                e.sug.join(", ").into()
            },
            page: e.page as i32,
            dimmed: e.dismissed,
            kind: e.kind.clone().into(),
            bar: kind_bar(&e.kind),
            desc: desc_for(&e.kind, en).into(),
        });
    }
    st.borrow_mut().view = view;
    ui.set_errors(ModelRc::new(VecModel::from_slice(&rows)));
    ui.set_spell_status(format!("{} errores", rows.len()).into());
}

/// Prepara el escaneo TODO el documento; el Timer lo avanza por trozos.
/// Orden: desde la página visible (resultados útiles primero).
/// ponytail: troceado en hilo UI en vez de segundo hilo con otro Pdfium (se colgaba en bind)
fn start_scan(ui: &AppWindow, st: &mut State) {
    let total = st.total;
    if st.path.is_none() || total == 0 {
        return;
    }
    st.scan_id += 1;
    st.all_errs.clear();
    st.scan_queue = (st.page..total).chain(0..st.page).collect();
    st.scan_pos = 0;
    st.last_lang = ui.get_lang_idx();
    ui.set_errors(ModelRc::new(VecModel::from_slice(&[])));
    ui.set_scanning(true);
    ui.set_scan_progress(0);
    ui.set_scan_text("Escaneando…".into());
    ui.set_spell_status("…".into());
}

/// Avanza el escaneo con presupuesto de 80ms por tick para no trabar la UI.
/// Devuelve true si llegaron hits de la página visible (hay que re-subrayar).
fn scan_tick(ui: &AppWindow, st: &Rc<RefCell<State>>) -> bool {
    {
        let s = st.borrow();
        if s.scan_pos >= s.scan_queue.len() || !s.dicts_ready || s.path.is_none() {
            return false;
        }
    }
    let mode = ui.get_lang_idx();
    let t = std::time::Instant::now();
    let mut advanced = false;
    let mut touched = false;
    {
        let mut s = st.borrow_mut();
        let Some(path) = s.path.clone() else { return false };
        let cur = s.page;
        while s.scan_pos < s.scan_queue.len() && t.elapsed() < std::time::Duration::from_millis(80) {
            let i = s.scan_queue[s.scan_pos];
            s.scan_pos += 1;
            advanced = true;
            let hits = {
                let Ok(doc) = s.pdfium.load_pdf_from_file(&path, None) else { continue };
                let Ok(pg) = doc.pages().get(i as u16) else { continue };
                let words = extract_words(&pg);
                let mut h = vec![];
                for (wi, w) in check_words(&words, mode) {
                    let b = &words[wi];
                    let kind = kind_of(&w).to_string();
                    h.push(Misspelling { word: w, sug: vec![], page: i, x0: b.x0, y0: b.y0, x1: b.x1, y1: b.y1, dismissed: false, kind });
                }
                h
            };
            if hits.iter().any(|h| h.page == cur) {
                touched = true;
            }
            s.all_errs.extend(hits);
        }
        let (done, total) = (s.scan_pos as u32, s.scan_queue.len() as u32);
        let pct = if total == 0 { 100 } else { done * 100 / total };
        ui.set_scan_progress(pct as i32);
        if s.scan_pos >= s.scan_queue.len() {
            ui.set_scanning(false);
            ui.set_scan_text("".into());
        } else {
            ui.set_scan_text(format!("{pct}% · pág. {done} de {total}…").into());
        }
    }
    if advanced {
        push_errs(ui, st);
    }
    touched
}

/// Agrupa chars de Pdfium en palabras con su bounding box unión.
fn extract_words(page: &PdfPage) -> Vec<Word> {
    let text = match page.text() {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let mut words = vec![];
    let mut cur = String::new();
    let mut x0 = f32::MAX;
    let mut y0 = f32::MAX;
    let mut x1 = f32::MIN;
    let mut y1 = f32::MIN;
    let mut flush = |cur: &mut String, words: &mut Vec<Word>, x0: &mut f32, y0: &mut f32, x1: &mut f32, y1: &mut f32| {
        let t = cur.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-').to_string();
        if t.chars().any(|c| c.is_alphabetic()) {
            words.push(Word { text: t, x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
        }
        cur.clear();
        *x0 = f32::MAX; *y0 = f32::MAX; *x1 = f32::MIN; *y1 = f32::MIN;
    };
    for ch in text.chars().iter() {
        let c = match ch.unicode_char() {
            Some(c) => c,
            None => continue,
        };
        if c.is_whitespace() {
            if !cur.is_empty() {
                flush(&mut cur, &mut words, &mut x0, &mut y0, &mut x1, &mut y1);
            }
            continue;
        }
        let r = ch.loose_bounds().or_else(|_| ch.tight_bounds());
        let r = match r {
            Ok(r) => r,
            Err(_) => continue,
        };
        // ponytail: unión simple de boxes, sin shaping tipográfico; refinar si el subrayado se desalinea
        x0 = x0.min(r.left().value);
        y0 = y0.min(r.bottom().value);
        x1 = x1.max(r.right().value);
        y1 = y1.max(r.top().value);
        cur.push(c);
    }
    if !cur.is_empty() {
        flush(&mut cur, &mut words, &mut x0, &mut y0, &mut x1, &mut y1);
    }
    words
}

/// Subrayado rojo quemado en el bitmap (siempre alineado, sin mates de layout).
/// Dibuja 3px al pie del box, mezcla 65% rojo #d32f2f sobre el fondo.
fn underline(img: &mut image::RgbaImage, x: f32, y: f32, w: f32) {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let (x0, y0) = (x.round() as i64, y.round() as i64);
    for dx in 0..(w.round() as i64).max(4) {
        for dy in 0..3i64 {
            let (px, py) = (x0 + dx, y0 + dy);
            if px < 0 || py < 0 || px >= iw || py >= ih {
                continue;
            }
            let p = img.get_pixel_mut(px as u32, py as u32);
            p[0] = (p[0] as u16 * 35 / 100 + 211 * 65 / 100) as u8;
            p[1] = (p[1] as u16 * 35 / 100 + 47 * 65 / 100) as u8;
            p[2] = (p[2] as u16 * 35 / 100 + 47 * 65 / 100) as u8;
        }
    }
}

fn wash(img: &mut image::RgbaImage, x: f32, y: f32, w: f32, h: f32) {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    for dx in 0..(w.round() as i64).max(4) {
        for dy in 0..(h.round() as i64).max(1) {
            let (px, py) = (x.round() as i64 + dx, y.round() as i64 + dy);
            if px < 0 || py < 0 || px >= iw || py >= ih {
                continue;
            }
            let p = img.get_pixel_mut(px as u32, py as u32);
            p[0] = (p[0] as u16 * 45 / 100 + 255 * 55 / 100) as u8;
            p[1] = (p[1] as u16 * 45 / 100 + 235 * 55 / 100) as u8;
            p[2] = (p[2] as u16 * 45 / 100 + 59 * 55 / 100) as u8;
        }
    }
}
fn pdf_to_image(x0: f32, y0: f32, x1: f32, y1: f32, pw: f32, ph: f32, iw: u32, ih: u32) -> (f32, f32, f32, f32) {
    let sx = iw as f32 / pw;
    let sy = ih as f32 / ph;
    let x = x0 * sx;
    let w = (x1 - x0) * sx;
    let y = (ph - y1) * sy;
    let h = (y1 - y0) * sy;
    (x, y, w, h)
}

/// Inyecta una nota nativa (sticky-note) en el box del error y guarda
/// en `{nombre}_anotado.pdf`. Nunca sobreescribe el original.
fn annotate(st: &State, idx: usize, text: &str) -> anyhow::Result<PathBuf> {
    let e = st.all_errs.get(idx).context("hallazgo no válido")?;
    let src = st.path.clone().context("sin documento")?;
    let text = if text.trim().is_empty() {
        format!("Revisar: {} (sugerencias: {})", e.word, e.sug.join(", "))
    } else {
        text.to_string()
    };
    let doc = st.pdfium.load_pdf_from_file(&src, None).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut pg = doc.pages().get(e.page as u16).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut ann = pg.annotations_mut().create_text_annotation(&text).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ann.set_bounds(PdfRect::new_from_values(e.y0, e.x0, e.y1, e.x1))
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("doc");
    let out = src.with_file_name(format!("{stem}_anotado.pdf"));
    doc.save_to_file(&out).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(out)
}

fn show(ui: &AppWindow, st: &mut State, page: u32) {
    let Some(path) = st.path.clone() else { return };
    let res: anyhow::Result<(Image, u32, Vec<Word>, f32, f32, u32, u32)> = (|| {
        let doc = st.pdfium.load_pdf_from_file(&path, None).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let pages = doc.pages();
        let total = pages.len() as u32;
        let pg = pages.get(page as u16).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let (pw, ph) = (pg.width().value, pg.height().value);
        let config = PdfRenderConfig::new().set_target_width(1400);
        let mut rgba = pg.render_with_config(&config).map_err(|e| anyhow::anyhow!("{e:?}"))?
            .as_image().as_rgba8().context("pdfium no devolvió RGBA")?.clone();
        let (iw, ih) = (rgba.width(), rgba.height());
        for h in st.all_errs.iter().filter(|e| e.page == page && !e.dismissed) {
            let (x, y, w, hh) = pdf_to_image(h.x0, h.y0, h.x1, h.y1, pw, ph, iw, ih);
            underline(&mut rgba, x, y + hh - 3.0, w);
        }
        // flash amarillo 5s del error clicado
        if let Some((fi, t)) = &st.flash {
            if t.elapsed() < std::time::Duration::from_secs(5) {
                if let Some(h) = st.all_errs.get(*fi) {
                    if h.page == page && !h.dismissed {
                        let (x, y, w, hh) = pdf_to_image(h.x0, h.y0, h.x1, h.y1, pw, ph, iw, ih);
                        wash(&mut rgba, x, y, w, hh);
                    }
                }
            }
        }
        let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), iw, ih));
        let words = extract_words(&pg);
        Ok((img, total, words, pw, ph, iw, ih))
    })();
    match res {
        Ok((img, total, words, pw, ph, iw, ih)) => {
            st.page = page;
            st.total = total;
            st.page_w = pw;
            st.page_h = ph;
            st.img_w = iw;
            st.img_h = ih;
            ui.set_page_image(img);
            ui.set_has_doc(true);
            ui.set_page_label(format!("Pág {} de {}", page + 1, total).into());
            ui.set_word_count(words.len() as i32);
            let sample: Vec<&str> = words.iter().take(12).map(|w| w.text.as_str()).collect();
            ui.set_word_sample(sample.join(" ").into());
            ui.set_status(format!("{} — {} págs. · {} palabras en pág. {}", path.display(), total, words.len(), page + 1).into());
            st.words = words;
        }
        Err(e) => ui.set_status(format!("Error: {e:#}").into()),
    }
}

fn load_pdfium() -> Pdfium {
    let lib = if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    };
    let mut cands = vec![];
    if let Ok(p) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        cands.push(PathBuf::from(p).join(lib));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            cands.push(d.join(lib));
        }
    }
    cands.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/pdfium")
            .join(lib),
    );
    for c in cands {
        if c.is_file() {
            if let Ok(b) = Pdfium::bind_to_library(&c) {
                return Pdfium::new(b);
            }
        }
    }
    Pdfium::default()
}

fn main() -> anyhow::Result<()> {
    let ui = AppWindow::new()?;
    if std::env::var("PDF_DARK").is_ok() {
        ui.set_is_dark(true);
    }
    let (tx, rx) = channel::<SpellMsg>();
    let st = Rc::new(RefCell::new(State {
        pdfium: load_pdfium(),
        path: None,
        page: 0,
        total: 0,
        words: vec![],
        page_w: 0.0,
        page_h: 0.0,
        img_w: 0,
        img_h: 0,
        all_errs: vec![],
        view: vec![],
        flash: None,
        scan_id: 0,
        scan_queue: vec![],
        scan_pos: 0,
        dicts_ready: false,
        tx,
        last_lang: 0,
    }));

    // recoge respuestas de los hilos + detecta cambio de idioma
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, std::time::Duration::from_millis(250), {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            if st.borrow().path.is_some() && ui.get_lang_idx() != st.borrow().last_lang {
                start_scan(&ui, &mut st.borrow_mut());
            }
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    SpellEvent::DictsReady => {
                        st.borrow_mut().dicts_ready = true;
                        ui.set_dicts_ready(true);
                    }
                    SpellEvent::Sug(gen, wi, sug) => {
                        if gen != st.borrow().scan_id {
                            continue;
                        }
                        let ok = {
                            let mut s = st.borrow_mut();
                            if let Some(e) = s.all_errs.get_mut(wi) {
                                e.sug = sug;
                                true
                            } else {
                                false
                            }
                        };
                        if ok {
                            push_errs(&ui, &st);
                            if let Some(e) = st.borrow().all_errs.get(wi) {
                                ui.set_status(format!("{} → {}", e.word, e.sug.join(", ")).into());
                            }
                        }
                    }
                }
            }
            if scan_tick(&ui, &st) {
                let pg = st.borrow().page;
                show(&ui, &mut st.borrow_mut(), pg);
            }
            // apaga el flash amarillo a los 5s
            if st.borrow().flash.is_some_and(|(_, t)| t.elapsed() > std::time::Duration::from_secs(5)) {
                st.borrow_mut().flash = None;
                let pg = st.borrow().page;
                show(&ui, &mut st.borrow_mut(), pg);
            }
        }
    });

    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_open_pdf(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let Some(file) = rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .pick_file()
            else {
                return;
            };
            ui.set_status("Cargando…".into());
            st.borrow_mut().path = Some(file);
            show(&ui, &mut st.borrow_mut(), 0);
            start_scan(&ui, &mut st.borrow_mut());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_prev_page(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let cur = st.borrow().page;
            if cur > 0 {
                show(&ui, &mut st.borrow_mut(), cur - 1);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_next_page(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let (cur, total) = (st.borrow().page, st.borrow().total);
            if cur + 1 < total {
                show(&ui, &mut st.borrow_mut(), cur + 1);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_error_clicked(move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let gi = match st.borrow().view.get(i as usize) {
                Some(g) => *g,
                None => return,
            };
            let (word, pg, has_sug) = match st.borrow().all_errs.get(gi) {
                Some(e) => (e.word.clone(), e.page, !e.sug.is_empty()),
                None => return,
            };
            st.borrow_mut().flash = Some((gi, std::time::Instant::now()));
            if pg != st.borrow().page {
                show(&ui, &mut st.borrow_mut(), pg); // salto + flash amarillo
            } else {
                let pg = st.borrow().page;
                show(&ui, &mut st.borrow_mut(), pg);
            }
            if has_sug {
                if let Some(e) = st.borrow().all_errs.get(gi) {
                    ui.set_status(format!("{} → {}", e.word, e.sug.join(", ")).into());
                }
                return;
            }
            let (mode, tx, gen) = (ui.get_lang_idx(), st.borrow().tx.clone(), st.borrow().scan_id);
            ui.set_status(format!("Buscando sugerencias para {word}…").into());
            std::thread::spawn(move || {
                let sug = suggest_for(&word, mode);
                let _ = tx.send(SpellEvent::Sug(gen, gi, sug));
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_theme_toggle(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_is_dark(!ui.get_is_dark());
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_omit_clicked(move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let pg = {
                let mut s = st.borrow_mut();
                let gi = match s.view.get(i as usize) {
                    Some(g) => *g,
                    None => return,
                };
                match s.all_errs.get_mut(gi) {
                    Some(e) => {
                        e.dismissed = !e.dismissed;
                        s.page
                    }
                    None => return,
                }
            };
            push_errs(&ui, &st);
            show(&ui, &mut st.borrow_mut(), pg); // refresca subrayados
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_note_clicked(move |i| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_selected_note(if ui.get_selected_note() == i { -1 } else { i });
                ui.set_note_text("".into());
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_save_note(move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let gi = match st.borrow().view.get(i as usize) {
                Some(g) => *g,
                None => return,
            };
            let text = ui.get_note_text().to_string();
            match annotate(&st.borrow(), gi, &text) {
                Ok(out) => ui.set_status(format!("Nota guardada en {} (reabre ese archivo para verla)", out.display()).into()),
                Err(e) => ui.set_status(format!("Error al anotar: {e:#}").into()),
            }
            ui.set_selected_note(-1);
        });
    }
    {
        let ui_weak = ui.as_weak();
        let st = st.clone();
        ui.on_filter_set(move |f| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_err_filter(f);
                push_errs(&ui, &st);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_lang_set(move |i| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_lang_idx(i);
            }
        });
    }

    // ponytail: parsear .dic/.aff tarda segundos en debug; se adelanta mientras el usuario elige archivo
    std::thread::spawn({
        let tx = st.borrow().tx.clone();
        move || {
            let _ = dict_cached("es_ES");
            let _ = dict_cached("en_US");
            let _ = tx.send(SpellEvent::DictsReady);
        }
    });

    // ponytail: abrir por argv evita el diálogo para pruebas/demos; quitar si molesta
    if let Some(arg) = std::env::args().nth(1) {
        let p = PathBuf::from(&arg);
        if p.is_file() {
            st.borrow_mut().path = Some(p);
            show(&ui, &mut st.borrow_mut(), 0);
            start_scan(&ui, &mut st.borrow_mut());
        }
    }

    ui.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_to_image_flip_y() {
        // pág 100x200pts -> img 1000x2000px; box inferior-izq (10,20)-(30,40)
        let (x, y, w, h) = pdf_to_image(10.0, 20.0, 30.0, 40.0, 100.0, 200.0, 1000, 2000);
        assert!((x - 100.0).abs() < 0.01 && (w - 200.0).abs() < 0.01);
        assert!((y - 1600.0).abs() < 0.01 && (h - 200.0).abs() < 0.01);
    }
    #[test]
    fn kinds() {
        assert_eq!(kind_of("herror"), "Error");
        assert_eq!(kind_of("Madrid"), "Mayúscula");
        assert_eq!(kind_of("ADSL"), "Sigla");
        assert_eq!(kind_of("a"), "Error");
    }
    #[test]
    fn underline_paints_red() {
        let mut img = image::RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        underline(&mut img, 1.0, 1.0, 5.0);
        let p = img.get_pixel(2, 2);
        assert!(p[0] > 200 && p[1] < 150 && p[2] < 150);
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 255, 255, 255]));
    }
    #[test]
    fn annotate_creates_sibling_file() {
        // ponytail: usa /tmp/doc3.pdf generado en dev; si falta, el test falla explícito
        let src = PathBuf::from("/tmp/doc3.pdf");
        assert!(src.is_file(), "falta /tmp/doc3.pdf de prueba");
        let (tx, _rx) = channel::<SpellMsg>();
        let st = State {
            pdfium: load_pdfium(), path: Some(src), page: 0, total: 3,
            words: vec![], page_w: 0.0, page_h: 0.0, img_w: 0, img_h: 0,
            all_errs: vec![Misspelling { word: "herror".into(), sug: vec!["error".into()], page: 0, x0: 72.0, y0: 700.0, x1: 130.0, y1: 712.0, dismissed: false, kind: "Error".into() }],
            scan_id: 0, scan_queue: vec![], scan_pos: 0, dicts_ready: true, tx, last_lang: 0, flash: None, view: vec![],
        };
        let out = annotate(&st, 0, "nota de prueba").unwrap();
        assert_eq!(out.file_name().unwrap(), "doc3_anotado.pdf");
        assert!(out.metadata().unwrap().len() > 500);
    }
}
