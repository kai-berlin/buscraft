//! Import of KNX XML exchange files (the published KNX project schema):
//! product databases (.knxprod) and projects (.knxproj, optionally password protected).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use std::sync::{Arc, Mutex};

use roxmltree::{Document, Node as XNode};

use crate::knxapp::*;
use crate::model::*;

// ------------------------------------------------------------------ translations and helpers

#[derive(Default)]
pub(crate) struct Tr(HashMap<(String, String), String>);

impl Tr {
    pub(crate) fn load(doc: &Document, lang: &str) -> Tr {
        let mut t = Tr::default();
        let prefix = &lang[..lang.len().min(2)];
        let langs: Vec<XNode> = doc.descendants().filter(|n| n.has_tag_name("Language")).collect();
        let pick = langs
            .iter()
            .find(|n| n.attribute("Identifier") == Some(lang))
            .or_else(|| langs.iter().find(|n| n.attribute("Identifier").map(|i| i.starts_with(prefix)).unwrap_or(false)));
        if let Some(l) = pick {
            for el in l.descendants().filter(|n| n.has_tag_name("TranslationElement")) {
                let Some(id) = el.attribute("RefId") else { continue };
                for tr in el.children().filter(|n| n.has_tag_name("Translation")) {
                    if let (Some(a), Some(x)) = (tr.attribute("AttributeName"), tr.attribute("Text")) {
                        t.0.insert((id.to_string(), a.to_string()), x.to_string());
                    }
                }
            }
        }
        t
    }
    pub(crate) fn get(&self, n: XNode, attr: &str) -> String {
        let id = n.attribute("Id").unwrap_or("");
        self.0.get(&(id.to_string(), attr.to_string())).cloned().or_else(|| n.attribute(attr).map(|s| s.to_string())).unwrap_or_default()
    }
}

/// Replace `{{0:default}}` style placeholders by their default text.
pub(crate) fn clean_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let inner = &after[..j];
                if let Some(k) = inner.find(':') {
                    out.push_str(inner[k + 1..].trim());
                }
                rest = &after[j + 2..];
            }
            None => rest = after,
        }
    }
    out.push_str(rest);
    out.replace("\r\n", " ").replace('\n', " ").trim().to_string()
}

fn flag(n: XNode, attr: &str) -> Option<bool> {
    n.attribute(attr).map(|v| v == "Enabled" || v == "true")
}

/// "DPST-9-1" → "9.001 temperature (°C)"-style text using the master data names.
pub(crate) fn dpt_text(raw: &str, names: &HashMap<String, String>) -> String {
    let first = raw.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = first.split('-').collect();
    let code = match parts.as_slice() {
        ["DPST", a, b] => format!("{a}.{:03}", b.parse::<u32>().unwrap_or(0)),
        ["DPT", a] => a.to_string(),
        _ => return first.to_string(),
    };
    match names.get(first) {
        Some(n) if !n.is_empty() => format!("{code} {n}"),
        _ => code,
    }
}

fn read_entry<R: Read + Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s.trim_start_matches('\u{feff}').to_string())
}

pub(crate) fn parse(text: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| format!("XML error: {e}"))
}

// ------------------------------------------------------------------ application program parsing

pub fn parse_application(text: &str, lang: &str, dpt_names: &HashMap<String, String>) -> Result<KnxApp, String> {
    let doc = parse(text)?;
    let tr = Tr::load(&doc, lang);
    let app = doc.descendants().find(|n| n.has_tag_name("ApplicationProgram")).ok_or("No application program in file")?;
    let id = app.attribute("Id").unwrap_or("").to_string();
    let ver: u32 = app.attribute("ApplicationVersion").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut notes = vec![];

    // Parameter types
    let mut types = HashMap::new();
    for pt in doc.descendants().filter(|n| n.has_tag_name("ParameterType")) {
        let Some(tid) = pt.attribute("Id") else { continue };
        let Some(def) = pt.children().find(|c| c.is_element()) else { continue };
        let t = match def.tag_name().name() {
            "TypeRestriction" => PType::Enum(
                def.children()
                    .filter(|c| c.has_tag_name("Enumeration"))
                    .map(|e| (e.attribute("Value").unwrap_or("").to_string(), clean_text(&tr.get(e, "Text"))))
                    .collect(),
            ),
            "TypeNumber" | "TypeTime" => {
                let float = def.attribute("Type").map(|t| t.contains("float")).unwrap_or(false);
                let min = def.attribute("minInclusive").and_then(num).unwrap_or(0.0);
                let max = def.attribute("maxInclusive").and_then(num).unwrap_or(65535.0);
                if float { PType::Float { min, max } } else { PType::Int { min, max } }
            }
            "TypeFloat" => PType::Float {
                min: def.attribute("minInclusive").and_then(num).unwrap_or(-670760.0),
                max: def.attribute("maxInclusive").and_then(num).unwrap_or(670760.0),
            },
            "TypeNone" | "TypePicture" => PType::None,
            _ => PType::Text,
        };
        types.insert(tid.to_string(), t);
    }

    // Parameters (also those inside unions)
    let mut base: HashMap<&str, XNode> = HashMap::new();
    for p in doc.descendants().filter(|n| n.has_tag_name("Parameter")) {
        if let Some(pid) = p.attribute("Id") {
            base.insert(pid, p);
        }
    }
    let access_of = |s: Option<&str>| match s {
        Some("None") => Some(0u8),
        Some("Read") => Some(1),
        Some("ReadWrite") => Some(2),
        _ => None,
    };
    let mut params = HashMap::new();
    for r in doc.descendants().filter(|n| n.has_tag_name("ParameterRef")) {
        let (Some(rid), Some(pid)) = (r.attribute("Id"), r.attribute("RefId")) else { continue };
        let Some(p) = base.get(pid) else { continue };
        let rtext = tr.get(r, "Text");
        let text = if rtext.is_empty() { tr.get(*p, "Text") } else { rtext };
        let text = if text.is_empty() { p.attribute("Name").unwrap_or("").to_string() } else { text };
        let unit = {
            let t = tr.get(r, "SuffixText");
            if t.is_empty() { tr.get(*p, "SuffixText") } else { t }
        };
        params.insert(
            rid.to_string(),
            PDef {
                text: clean_text(&text),
                ptype: p.attribute("ParameterType").unwrap_or("").to_string(),
                value: r.attribute("Value").or(p.attribute("Value")).unwrap_or("").to_string(),
                access: access_of(r.attribute("Access")).or(access_of(p.attribute("Access"))).unwrap_or(2),
                unit,
            },
        );
    }

    // Group objects
    let mut cbase: HashMap<&str, XNode> = HashMap::new();
    for c in doc.descendants().filter(|n| n.has_tag_name("ComObject")) {
        if let Some(cid) = c.attribute("Id") {
            cbase.insert(cid, c);
        }
    }
    let mut coms = HashMap::new();
    for r in doc.descendants().filter(|n| n.has_tag_name("ComObjectRef")) {
        let (Some(rid), Some(cid)) = (r.attribute("Id"), r.attribute("RefId")) else { continue };
        let Some(c) = cbase.get(cid) else { continue };
        let pick = |attr: &str| {
            let v = tr.get(r, attr);
            if v.is_empty() { tr.get(*c, attr) } else { v }
        };
        let f = |attr: &str| flag(r, attr).or(flag(*c, attr)).unwrap_or(false);
        let dpt_raw = r.attribute("DatapointType").or(c.attribute("DatapointType")).unwrap_or("");
        let mut text = pick("Text");
        if text.is_empty() {
            text = c.attribute("Name").unwrap_or("").to_string();
        }
        coms.insert(
            rid.to_string(),
            CoDef {
                number: c.attribute("Number").and_then(|n| n.parse().ok()).unwrap_or(0),
                text: clean_text(&text),
                function: clean_text(&pick("FunctionText")),
                size: r.attribute("ObjectSize").or(c.attribute("ObjectSize")).unwrap_or("").to_lowercase(),
                dpt: dpt_text(dpt_raw, dpt_names),
                flags: Flags { c: f("CommunicationFlag"), r: f("ReadFlag"), w: f("WriteFlag"), t: f("TransmitFlag"), u: f("UpdateFlag") },
            },
        );
    }

    // Dynamic structure
    let mut unsupported = 0usize;
    let dynamic = match app.descendants().find(|n| n.has_tag_name("Dynamic")) {
        Some(d) => dyn_children(d, &tr, &mut unsupported),
        None => vec![],
    };
    if unsupported > 0 {
        notes.push(format!("{unsupported} advanced parts of this application (modules or calculations) are not shown"));
    }
    let dynamic = if dynamic.is_empty() {
        if !params.is_empty() {
            notes.push("No page structure found – all parameters are listed on one page".into());
        }
        let mut ids: Vec<&String> = params.keys().collect();
        ids.sort();
        let mut c: Vec<Node> = ids.into_iter().map(|i| Node::Param(i.clone())).collect();
        let mut coms_sorted: Vec<(&String, &CoDef)> = coms.iter().collect();
        coms_sorted.sort_by_key(|(_, c)| c.number);
        c.extend(coms_sorted.into_iter().map(|(i, _)| Node::Com(i.clone())));
        vec![Node::Block { text: "General".into(), inline: false, children: c }]
    } else {
        dynamic
    };

    let load = Some(crate::knxload::parse_load(&doc, app));
    // Parameter calculations (scripts). Variables are named by AliasName or the parameter's Name.
    let param_name = |r: &str| -> String {
        let pid = doc.descendants().find(|n| n.has_tag_name("ParameterRef") && n.attribute("Id") == Some(r)).and_then(|n| n.attribute("RefId")).unwrap_or("");
        base.get(pid).and_then(|p| p.attribute("Name")).unwrap_or("").to_string()
    };
    let mut calcs = vec![];
    for pc in doc.descendants().filter(|n| n.has_tag_name("ParameterCalculation")) {
        if pc.attribute("Language").map(|l| !l.eq_ignore_ascii_case("JavaScript")).unwrap_or(false) {
            notes.push(format!("Calculation \"{}\" uses an unsupported language", pc.attribute("Name").unwrap_or("")));
            continue;
        }
        let script = pc.children().find(|c| c.has_tag_name("LRTransformation")).and_then(|c| c.text()).unwrap_or("").to_string();
        let refs = |tag: &str| -> Vec<(String, String)> {
            pc.children()
                .find(|c| c.has_tag_name(tag))
                .map(|l| {
                    l.children()
                        .filter(|c| c.has_tag_name("ParameterRefRef"))
                        .filter_map(|c| {
                            let r = c.attribute("RefId")?.to_string();
                            let alias = c.attribute("AliasName").map(|s| s.to_string()).unwrap_or_else(|| param_name(&r));
                            Some((r, alias))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let func = pc.attribute("LRTransformationFunc").map(|s| s.to_string());
        calcs.push(crate::knxapp::Calc { name: pc.attribute("Name").unwrap_or("").to_string(), script, inputs: refs("LParameters"), outputs: refs("RParameters"), func });
    }
    Ok(KnxApp {
        load,
        calcs,
        script: doc.descendants().filter(|n| n.has_tag_name("Script")).filter_map(|n| n.text()).collect::<Vec<_>>().join("\n"),
        source: String::new(),
        format: 3,
        name: clean_text(&tr.get(app, "Name")),
        version: format!("{}.{}", ver >> 4, ver & 0x0f),
        id,
        types,
        params,
        coms,
        dynamic,
        notes,
    })
}

fn dyn_children(n: XNode, tr: &Tr, unsupported: &mut usize) -> Vec<Node> {
    let mut out = vec![];
    for c in n.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "ChannelIndependentBlock" => out.push(Node::Channel { text: String::new(), children: dyn_children(c, tr, unsupported) }),
            "Channel" => out.push(Node::Channel { text: clean_text(&tr.get(c, "Text")), children: dyn_children(c, tr, unsupported) }),
            "ParameterBlock" | "ComObjectParameterBlock" => {
                let mut text = tr.get(c, "Text");
                if text.is_empty() {
                    text = c.attribute("Name").unwrap_or("").to_string();
                }
                out.push(Node::Block {
                    text: clean_text(&text),
                    inline: c.attribute("Inline") == Some("true"),
                    children: dyn_children(c, tr, unsupported),
                });
            }
            "ParameterRefRef" => {
                if let Some(r) = c.attribute("RefId") {
                    out.push(Node::Param(r.to_string()));
                }
            }
            "ComObjectRefRef" => {
                if let Some(r) = c.attribute("RefId") {
                    out.push(Node::Com(r.to_string()));
                }
            }
            "ParameterSeparator" => out.push(Node::Sep(clean_text(&tr.get(c, "Text")))),
            "choose" => {
                let whens = c
                    .children()
                    .filter(|w| w.has_tag_name("when"))
                    .map(|w| When {
                        test: w.attribute("test").unwrap_or("").split_whitespace().map(|s| s.to_string()).collect(),
                        default: w.attribute("default") == Some("true"),
                        children: dyn_children(w, tr, unsupported),
                    })
                    .collect();
                out.push(Node::Choose { param: c.attribute("ParamRefId").unwrap_or("").to_string(), whens });
            }
            "Assign" => {
                if let Some(target) = c.attribute("TargetParamRefRef") {
                    out.push(Node::Assign {
                        target: target.to_string(),
                        source: c.attribute("SourceParamRefRef").map(|s| s.to_string()),
                        value: c.attribute("Value").map(|s| s.to_string()),
                    });
                }
            }
            "Module" | "Repeat" | "Rename" | "Button" => *unsupported += 1,
            _ => out.extend(dyn_children(c, tr, unsupported)),
        }
    }
    out
}

// ------------------------------------------------------------------ manufacturer data

#[derive(Default)]
pub struct MfrData {
    pub manufacturers: HashMap<String, String>,
    pub dpt_names: HashMap<String, String>,
    /// Product id → (text, order number, hardware id)
    pub products: HashMap<String, (String, String, String)>,
    /// Hardware2Program id → (application id, medium)
    pub hw2prog: HashMap<String, (String, Medium)>,
    pub apps: HashMap<String, KnxApp>,
    /// (catalog path, name, product id, hw2prog id, description)
    pub catalog: Vec<(String, String, String, String, String)>,
    pub warnings: Vec<String>,
    pub masks: Vec<crate::knxload::MaskInfo>,
}

pub(crate) fn medium_of(s: &str) -> Medium {
    match s.split_whitespace().next().unwrap_or("") {
        "MT-2" => Medium::RF,
        "MT-5" => Medium::IP,
        _ => Medium::TP,
    }
}

fn parse_master(text: &str, out: &mut MfrData) {
    let Ok(doc) = parse(text) else { return };
    out.masks = crate::knxload::parse_masks(&doc);
    for m in doc.descendants().filter(|n| n.has_tag_name("Manufacturer")) {
        if let (Some(id), Some(name)) = (m.attribute("Id"), m.attribute("Name")) {
            out.manufacturers.insert(id.to_string(), name.to_string());
        }
    }
    for s in doc.descendants().filter(|n| n.has_tag_name("DatapointSubtype") || n.has_tag_name("DatapointType")) {
        if let (Some(id), Some(t)) = (s.attribute("Id"), s.attribute("Text")) {
            out.dpt_names.insert(id.to_string(), t.to_string());
        }
    }
}

fn parse_hardware(text: &str, lang: &str, out: &mut MfrData) -> Result<(), String> {
    let doc = parse(text)?;
    let tr = Tr::load(&doc, lang);
    for hw in doc.descendants().filter(|n| n.has_tag_name("Hardware") && n.attribute("Id").is_some()) {
        let hid = hw.attribute("Id").unwrap_or("").to_string();
        for p in hw.descendants().filter(|n| n.has_tag_name("Product")) {
            let Some(pid) = p.attribute("Id") else { continue };
            out.products.insert(pid.to_string(), (tr.get(p, "Text"), p.attribute("OrderNumber").unwrap_or("").to_string(), hid.clone()));
        }
        for h2p in hw.descendants().filter(|n| n.has_tag_name("Hardware2Program")) {
            let Some(id) = h2p.attribute("Id") else { continue };
            let app = h2p.descendants().find(|n| n.has_tag_name("ApplicationProgramRef")).and_then(|n| n.attribute("RefId")).unwrap_or("");
            out.hw2prog.insert(id.to_string(), (app.to_string(), medium_of(h2p.attribute("MediumTypes").unwrap_or(""))));
        }
    }
    Ok(())
}

fn parse_catalog(text: &str, lang: &str, out: &mut MfrData) -> Result<(), String> {
    let doc = parse(text)?;
    let tr = Tr::load(&doc, lang);
    for item in doc.descendants().filter(|n| n.has_tag_name("CatalogItem")) {
        let mut path = vec![];
        let mut p = item.parent();
        while let Some(x) = p {
            if x.has_tag_name("CatalogSection") {
                path.push(tr.get(x, "Name"));
            }
            p = x.parent();
        }
        path.reverse();
        out.catalog.push((
            path.join(" › "),
            tr.get(item, "Name"),
            item.attribute("ProductRefId").unwrap_or("").to_string(),
            item.attribute("Hardware2ProgramRefId").unwrap_or("").to_string(),
            tr.get(item, "VisibleDescription"),
        ));
    }
    Ok(())
}

/// Read manufacturer data (master data, hardware, catalog, applications) from an open archive.
fn read_mfr<R: Read + Seek>(z: &mut zip::ZipArchive<R>, lang: &str, progress: &Progress) -> MfrData {
    let mut out = MfrData::default();
    if let Some(m) = read_entry(z, "knx_master.xml") {
        progress.set("Reading master data…");
        parse_master(&m, &mut out);
    }
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|s| s.to_string())).collect();
    for n in names.iter().filter(|n| n.starts_with("M-") && n.ends_with("/Hardware.xml")) {
        if let Some(t) = read_entry(z, n) {
            if let Err(e) = parse_hardware(&t, lang, &mut out) {
                out.warnings.push(format!("{n}: {e}"));
            }
        }
    }
    for n in names.iter().filter(|n| n.starts_with("M-") && n.ends_with("/Catalog.xml")) {
        if let Some(t) = read_entry(z, n) {
            if let Err(e) = parse_catalog(&t, lang, &mut out) {
                out.warnings.push(format!("{n}: {e}"));
            }
        }
    }
    let apps: Vec<&String> = names.iter().filter(|n| n.starts_with("M-") && n.contains("_A-") && n.ends_with(".xml")).collect();
    let total = apps.len();
    for (i, n) in apps.into_iter().enumerate() {
        progress.set(&format!("Reading application {} of {total}…", i + 1));
        if let Some(t) = read_entry(z, n) {
            match parse_application(&t, lang, &out.dpt_names) {
                Ok(a) => {
                    out.apps.insert(a.id.clone(), a);
                }
                Err(e) => out.warnings.push(format!("{n}: {e}")),
            }
        }
    }
    out
}

impl MfrData {
    pub(crate) fn mfr_name(&self, any_id: &str) -> String {
        let key = any_id.split('_').next().unwrap_or("");
        self.manufacturers.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    /// Catalog products for the library.
    pub fn products_list(&self) -> Vec<Product> {
        let mut out = vec![];
        let mut seen = std::collections::HashSet::new();
        let mut entries = self.catalog.clone();
        // Hardware/program pairs not listed in a catalog (projects carry only part of it).
        for (h2p, _) in &self.hw2prog {
            if entries.iter().any(|e| &e.3 == h2p) {
                continue;
            }
            let hid = h2p.rsplit_once("_HP-").map(|x| x.0).unwrap_or("");
            for (pid, (text, _, h)) in &self.products {
                if h == hid {
                    entries.push((String::new(), text.clone(), pid.clone(), h2p.clone(), String::new()));
                }
            }
        }
        for (path, name, pid, h2p, desc) in entries {
            if !seen.insert((pid.clone(), h2p.clone())) {
                continue;
            }
            let (ptext, order, _hid) = self.products.get(&pid).cloned().unwrap_or_default();
            let (app_id, medium) = self.hw2prog.get(&h2p).cloned().unwrap_or((String::new(), Medium::TP));
            let app = self.apps.get(&app_id);
            out.push(Product {
                manufacturer: self.mfr_name(&pid),
                name: if name.is_empty() { ptext } else { name },
                order_no: order,
                app_program: app.map(|a| format!("{} {}", a.name, a.version)).unwrap_or_default(),
                medium,
                category: if path.is_empty() { "Imported".into() } else { path },
                description: desc,
                params: vec![],
                objects: vec![],
                app_id: app.map(|a| a.id.clone()),
                imported: true,
            });
        }
        out.sort_by(|a, b| (&a.manufacturer, &a.category, &a.name).cmp(&(&b.manufacturer, &b.category, &b.name)));
        out
    }
}

// ------------------------------------------------------------------ public import API

#[derive(Clone, Default)]
pub struct Progress(Arc<Mutex<String>>);
impl Progress {
    pub fn set(&self, s: &str) {
        if let Ok(mut g) = self.0.lock() {
            *g = s.to_string();
        }
    }
    pub fn get(&self) -> String {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

pub struct ImportOut {
    pub masks: Vec<crate::knxload::MaskInfo>,
    pub products: Vec<Product>,
    pub apps: Vec<KnxApp>,
    pub project: Option<Project>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum ImportErr {
    NeedPassword(String),
    WrongPassword(String),
    Failed(String),
}

impl std::fmt::Display for ImportErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportErr::NeedPassword(n) => write!(f, "The project \"{n}\" is password protected"),
            ImportErr::WrongPassword(n) => write!(f, "Wrong password for \"{n}\""),
            ImportErr::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// The ZIP password used for projects of schema 21 and later (derived from the project password).
pub fn ets6_zip_password(password: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = password.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&utf16, b"21.project.ets.knx.org", 65536, &mut key);
    base64::engine::general_purpose::STANDARD.encode(key)
}

/// Import a .knxprod (products) or .knxproj (project and the products it uses).
/// `products_only`: for projects, skip the project data (e.g. when no password is known).
pub fn import(path: &Path, password: Option<&str>, products_only: bool, lang: &str, progress: &Progress) -> Result<ImportOut, ImportErr> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext.starts_with("vd") {
        return Err(ImportErr::Failed(
            "Older product databases (.vd1 – .vd5) are encrypted in a format this version cannot open. Please use the manufacturer's .knxprod file for this product instead.".into(),
        ));
    }
    let file = std::fs::File::open(path).map_err(|e| ImportErr::Failed(format!("Could not open the file: {e}")))?;
    let mut z = zip::ZipArchive::new(file).map_err(|e| ImportErr::Failed(format!("Not a valid KNX file: {e}")))?;
    progress.set("Reading products…");
    let mut mfr = read_mfr(&mut z, lang, progress);
    for a in mfr.apps.values_mut() {
        a.source = path.display().to_string();
    }
    let mut warnings = mfr.warnings.clone();
    let products = mfr.products_list();

    let mut project = None;
    if ext == "knxproj" && !products_only {
        progress.set("Reading project…");
        project = Some(crate::knxproj::read_project(&mut z, path, password, &mfr, &mut warnings)?);
    }
    if products.is_empty() && project.is_none() && mfr.apps.is_empty() {
        return Err(ImportErr::Failed("No products were found in this file".into()));
    }
    Ok(ImportOut { masks: mfr.masks.clone(), products, apps: mfr.apps.into_values().collect(), project, warnings })
}

pub(crate) fn project_name_hint<R: Read + Seek>(z: &mut zip::ZipArchive<R>, path: &Path) -> String {
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|s| s.to_string())).collect();
    for n in names.iter().filter(|n| n.starts_with("P-") && n.ends_with(".info")) {
        if let Some(t) = read_entry(z, n) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                if let Some(s) = v.get("ProjectName").and_then(|s| s.as_str()) {
                    return s.to_string();
                }
            }
        }
    }
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Project").to_string()
}

/// Read 0.xml and project.xml of the project, decrypting them when needed.
pub(crate) fn project_xml<R: Read + Seek>(z: &mut zip::ZipArchive<R>, path: &Path, password: Option<&str>) -> Result<(String, String, String), ImportErr> {
    let name_hint = project_name_hint(z, path);
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|s| s.to_string())).collect();
    if let Some(n) = names.iter().find(|n| n.starts_with("P-") && n.ends_with("/0.xml")) {
        let dir = n.trim_end_matches("0.xml").to_string();
        return Ok((read_entry(z, n).unwrap_or_default(), read_entry(z, &format!("{dir}project.xml")).unwrap_or_default(), name_hint));
    }
    let Some(n) = names.iter().find(|n| n.starts_with("P-") && n.ends_with(".zip")) else {
        return Err(ImportErr::Failed("No project data found in this file".into()));
    };
    let mut bytes = vec![];
    z.by_name(n).map_err(|e| ImportErr::Failed(e.to_string()))?.read_to_end(&mut bytes).map_err(|e| ImportErr::Failed(e.to_string()))?;
    let mut inner = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ImportErr::Failed(format!("Project data damaged: {e}")))?;
    if inner.by_name("0.xml").is_ok() {
        return Ok((read_entry(&mut inner, "0.xml").unwrap_or_default(), read_entry(&mut inner, "project.xml").unwrap_or_default(), name_hint));
    }
    let Some(pw) = password.filter(|p| !p.is_empty()) else { return Err(ImportErr::NeedPassword(name_hint)) };
    for c in [ets6_zip_password(pw), pw.to_string()] {
        let mut s = String::new();
        let ok = match inner.by_name_decrypt("0.xml", c.as_bytes()) {
            Ok(mut f) => f.read_to_string(&mut s).is_ok(),
            Err(_) => false,
        };
        if ok {
            let mut p = String::new();
            if let Ok(mut f) = inner.by_name_decrypt("project.xml", c.as_bytes()) {
                let _ = f.read_to_string(&mut p);
            }
            return Ok((s.trim_start_matches('\u{feff}').to_string(), p.trim_start_matches('\u{feff}').to_string(), name_hint));
        }
    }
    Err(ImportErr::WrongPassword(name_hint))
}
