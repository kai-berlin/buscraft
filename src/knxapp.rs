//! Application program definitions parsed from KNX product files, and their evaluation:
//! parameters, parameter types, group objects and the dynamic page structure
//! (parameter blocks, channels, choose/when conditions).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PType {
    Enum(Vec<(String, String)>),
    Int { min: f64, max: f64 },
    Float { min: f64, max: f64 },
    Text,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PDef {
    pub text: String,
    pub ptype: String,
    pub value: String,
    /// 0 = hidden, 1 = read only, 2 = editable
    pub access: u8,
    #[serde(default)]
    pub unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoDef {
    pub number: u16,
    pub text: String,
    pub function: String,
    pub size: String,
    pub dpt: String,
    pub flags: Flags,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Node {
    Block { text: String, inline: bool, children: Vec<Node> },
    Channel { text: String, children: Vec<Node> },
    Param(String),
    Com(String),
    Sep(String),
    Choose { param: String, whens: Vec<When> },
    /// Set a parameter (to a fixed value or another parameter's value) when reached.
    Assign { target: String, source: Option<String>, value: Option<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct When {
    pub test: Vec<String>,
    pub default: bool,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnxApp {
    pub id: String,
    pub name: String,
    pub version: String,
    pub types: HashMap<String, PType>,
    /// Keyed by ParameterRef id (base parameter merged with the reference's overrides).
    pub params: HashMap<String, PDef>,
    /// Keyed by ComObjectRef id (base object merged with the reference's overrides).
    pub coms: HashMap<String, CoDef>,
    pub dynamic: Vec<Node>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Data needed to download the application into a device.
    #[serde(default)]
    pub load: Option<crate::knxload::LoadInfo>,
    /// Parameter calculations (scripts computing hidden parameters).
    #[serde(default)]
    pub calcs: Vec<Calc>,
    /// Shared script of the application (functions used by calculations).
    #[serde(default)]
    pub script: String,
    /// File the application was imported from (for automatic re-import).
    #[serde(default)]
    pub source: String,
    /// Import format: 2 = Assign rules and calculations, 3 = union and default data for downloads.
    #[serde(default)]
    pub format: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calc {
    pub name: String,
    pub script: String,
    /// (parameter reference id, variable name)
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    /// Function in the application's shared script (called with input/output objects).
    #[serde(default)]
    pub func: Option<String>,
}

/// One page of the parameter dialog, after evaluating conditions.
#[derive(Clone, Debug)]
pub struct Page {
    pub text: String,
    pub depth: usize,
    pub header: bool,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Param(String),
    Sep(String),
    Heading(String),
}

pub struct Visible {
    pub pages: Vec<Page>,
    pub coms: Vec<String>,
    /// Every parameter reference reached on the active branches (also hidden ones).
    pub reached: std::collections::HashSet<String>,
}

pub fn num(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(h, 16).ok().map(|v| v as f64);
    }
    s.parse::<f64>().ok()
}

/// Does a `when test="…"` match the parameter value?
pub fn test_matches(tests: &[String], value: &str) -> bool {
    tests.iter().any(|t| {
        let (op, rhs) = if let Some(r) = t.strip_prefix("!=") {
            ("!=", r)
        } else if let Some(r) = t.strip_prefix("<=") {
            ("<=", r)
        } else if let Some(r) = t.strip_prefix(">=") {
            (">=", r)
        } else if let Some(r) = t.strip_prefix('<') {
            ("<", r)
        } else if let Some(r) = t.strip_prefix('>') {
            (">", r)
        } else if let Some(r) = t.strip_prefix('!') {
            ("!=", r)
        } else {
            ("=", t.as_str())
        };
        match (num(value), num(rhs)) {
            (Some(a), Some(b)) => match op {
                "!=" => a != b,
                "<=" => a <= b,
                ">=" => a >= b,
                "<" => a < b,
                ">" => a > b,
                _ => a == b,
            },
            _ => match op {
                "!=" => value != rhs,
                "=" => value == rhs,
                _ => false,
            },
        }
    })
}

impl KnxApp {
    pub fn value_of(&self, values: &BTreeMap<String, String>, param: &str) -> String {
        values.get(param).cloned().or_else(|| self.params.get(param).map(|p| p.value.clone())).unwrap_or_default()
    }

    /// Evaluate the dynamic structure with the given parameter values.
    pub fn visible(&self, values: &BTreeMap<String, String>) -> Visible {
        let mut v = Visible { pages: vec![], coms: vec![], reached: Default::default() };
        let mut current: Option<usize> = None;
        let mut vals = self.effective_values(values).0;
        self.walk(&self.dynamic, &mut vals, 0, &mut current, &mut v, 0);
        // Drop empty pages, but keep channel headers that still have pages below them.
        let pages = std::mem::take(&mut v.pages);
        let mut keep = vec![false; pages.len()];
        for i in (0..pages.len()).rev() {
            if !pages[i].header {
                keep[i] = !pages[i].items.is_empty() || (i + 1 < pages.len() && pages[i + 1].depth > pages[i].depth && keep[i + 1]);
            } else {
                keep[i] = (i + 1..pages.len()).take_while(|j| pages[*j].depth > pages[i].depth).any(|j| keep[j]);
            }
        }
        v.pages = pages.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect();
        let mut seen = std::collections::HashSet::new();
        v.coms.retain(|c| seen.insert(c.clone()));
        v.coms.sort_by_key(|c| self.coms.get(c).map(|x| x.number).unwrap_or(u16::MAX));
        v
    }

    fn walk(&self, nodes: &[Node], values: &mut BTreeMap<String, String>, depth: usize, current: &mut Option<usize>, out: &mut Visible, level: usize) {
        if level > 200 {
            return;
        }
        for n in nodes {
            match n {
                Node::Channel { text, children } => {
                    let saved = *current;
                    if text.is_empty() {
                        self.walk(children, values, depth, current, out, level + 1);
                    } else {
                        out.pages.push(Page { text: text.clone(), depth, header: true, items: vec![] });
                        *current = None;
                        self.walk(children, values, depth + 1, current, out, level + 1);
                    }
                    *current = saved;
                }
                Node::Block { text, inline, children } => {
                    if *inline && current.is_some() {
                        if !text.is_empty() {
                            out.pages[current.unwrap()].items.push(Item::Heading(text.clone()));
                        }
                        self.walk(children, values, depth, current, out, level + 1);
                    } else {
                        let saved = *current;
                        out.pages.push(Page { text: if text.is_empty() { "General".into() } else { text.clone() }, depth, header: false, items: vec![] });
                        *current = Some(out.pages.len() - 1);
                        self.walk(children, values, depth + 1, current, out, level + 1);
                        *current = saved;
                    }
                }
                Node::Param(id) => {
                    out.reached.insert(id.clone());
                    if self.params.get(id).map(|p| p.access > 0).unwrap_or(false) {
                        let i = match *current {
                            Some(i) => i,
                            None => {
                                out.pages.push(Page { text: "General".into(), depth, header: false, items: vec![] });
                                *current = Some(out.pages.len() - 1);
                                out.pages.len() - 1
                            }
                        };
                        out.pages[i].items.push(Item::Param(id.clone()));
                    }
                }
                Node::Sep(t) => {
                    if let Some(i) = *current {
                        out.pages[i].items.push(Item::Sep(t.clone()));
                    }
                }
                Node::Com(id) => out.coms.push(id.clone()),
                Node::Assign { target, source, value } => {
                    if let Some(v) = source.as_ref().map(|s| self.value_of(values, s)).or_else(|| value.clone()) {
                        values.insert(target.clone(), v);
                    }
                }
                Node::Choose { param, whens } => {
                    let v = self.value_of(values, param);
                    let mut any = false;
                    for w in whens.iter().filter(|w| !w.default) {
                        if test_matches(&w.test, &v) {
                            any = true;
                            self.walk(&w.children, values, depth, current, out, level + 1);
                        }
                    }
                    if !any {
                        for w in whens.iter().filter(|w| w.default) {
                            self.walk(&w.children, values, depth, current, out, level + 1);
                        }
                    }
                }
            }
        }
    }

    /// Parameter values as the device should get them: the stored values plus everything the
    /// application sets by itself (Assign rules on the active branches, and calculations).
    /// Repeated until stable, since results can switch other branches. Returns the values and
    /// notes about calculations that could not be evaluated.
    pub fn effective_values(&self, stored: &BTreeMap<String, String>) -> (BTreeMap<String, String>, Vec<String>) {
        let mut vals = stored.clone();
        let mut notes = vec![];
        for round in 0..5 {
            let before = vals.clone();
            self.apply_assigns(&self.dynamic, &mut vals, 0);
            for c in &self.calcs {
                let (pre_in, pre_out, script) = match &c.func {
                    Some(f) => ("input.", "output.", format!("{}\n{f}(0, 0, 0);", self.script)),
                    None => ("", "", c.script.clone()),
                };
                let mut inputs = HashMap::new();
                for (r, alias) in &c.inputs {
                    let v = self.value_of(&vals, r);
                    inputs.insert(format!("{pre_in}{alias}"), num(&v).unwrap_or(0.0));
                }
                match crate::knxcalc::run(&script, &inputs) {
                    Ok(out) => {
                        for (r, alias) in &c.outputs {
                            if let Some(v) = out.get(&format!("{pre_out}{alias}")) {
                                vals.insert(r.clone(), crate::knxcalc::fmt_num(*v));
                            }
                        }
                    }
                    Err(e) => {
                        if round == 0 {
                            notes.push(format!("Calculation \"{}\" could not be evaluated ({e}); its parameters keep their stored values", c.name));
                        }
                    }
                }
            }
            if vals == before {
                break;
            }
        }
        (vals, notes)
    }

    fn apply_assigns(&self, nodes: &[Node], vals: &mut BTreeMap<String, String>, level: usize) {
        if level > 200 {
            return;
        }
        for n in nodes {
            match n {
                Node::Block { children, .. } | Node::Channel { children, .. } => self.apply_assigns(children, vals, level + 1),
                Node::Assign { target, source, value } => {
                    if let Some(v) = source.as_ref().map(|s| self.value_of(vals, s)).or_else(|| value.clone()) {
                        vals.insert(target.clone(), v);
                    }
                }
                Node::Choose { param, whens } => {
                    let v = self.value_of(vals, param);
                    let mut any = false;
                    for w in whens.iter().filter(|w| !w.default) {
                        if test_matches(&w.test, &v) {
                            any = true;
                            self.apply_assigns(&w.children, vals, level + 1);
                        }
                    }
                    if !any {
                        for w in whens.iter().filter(|w| w.default) {
                            self.apply_assigns(&w.children, vals, level + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// What is active with these settings: reached parameters and visible group objects.
    pub fn active(&self, values: &BTreeMap<String, String>) -> crate::knxload::Active {
        let v = self.visible(values);
        crate::knxload::Active { objects: v.coms.iter().filter_map(|c| self.coms.get(c).map(|x| (x.number, (x.flags, x.size.clone())))).collect(), reached: v.reached }
    }

    pub fn object_from(&self, id: &str, links: Vec<u32>) -> Option<GroupObject> {
        let c = self.coms.get(id)?;
        Some(GroupObject {
            number: c.number,
            name: c.text.clone(),
            function: c.function.clone(),
            description: String::new(),
            dpt: c.dpt.clone(),
            flags: c.flags,
            priority: "Low".into(),
            links,
            ref_id: id.to_string(),
            size: c.size.clone(),
        })
    }

    /// Build the device's group object list from the visible objects, keeping existing links.
    pub fn sync_objects(&self, dev: &mut Device) {
        let vis = self.visible(&dev.knx_values);
        let old: HashMap<String, GroupObject> = dev.objects.drain(..).map(|o| (o.ref_id.clone(), o)).collect();
        let mut out = vec![];
        for id in &vis.coms {
            let links = old.get(id).map(|o| o.links.clone()).unwrap_or_default();
            if let Some(mut o) = self.object_from(id, links) {
                if let Some(prev) = old.get(id) {
                    o.flags = prev.flags;
                }
                out.push(o);
            }
        }
        // Keep linked objects that became invisible, so no links are lost silently.
        for (id, o) in old {
            if !o.links.is_empty() && !out.iter().any(|x| x.ref_id == id) {
                out.push(o);
            }
        }
        out.sort_by_key(|o| o.number);
        dev.objects = out;
    }

    /// Resolve a (possibly shortened) reference id used inside project files.
    pub fn resolve(&self, r: &str, coms: bool) -> Option<String> {
        let full = format!("{}_{}", self.id, r);
        let has = |k: &str| if coms { self.coms.contains_key(k) } else { self.params.contains_key(k) };
        if has(r) {
            Some(r.to_string())
        } else if has(&full) {
            Some(full)
        } else {
            None
        }
    }
}
