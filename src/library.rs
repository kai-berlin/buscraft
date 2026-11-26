//! The product library: imported products (catalog entries) and application program
//! definitions, stored in the app's data folder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::knxapp::KnxApp;
use crate::model::Product;

pub struct Library {
    dir: Option<PathBuf>,
    pub products: Vec<Product>,
    cache: HashMap<String, Arc<KnxApp>>,
    pub masks: HashMap<String, crate::knxload::MaskInfo>,
}

fn safe(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

impl Library {
    /// `dir = None` keeps everything in memory (used for screenshots and tests).
    pub fn load(dir: Option<PathBuf>) -> Library {
        let products = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("catalog.json")).ok())
            .and_then(|s| serde_json::from_str::<Vec<Product>>(&s).ok())
            .unwrap_or_default();
        let masks = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("masks.json")).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Library { dir, products, cache: HashMap::new(), masks }
    }

    pub fn default_dir() -> PathBuf {
        crate::app::data_dir().join("library")
    }

    pub fn app(&mut self, id: &str) -> Option<Arc<KnxApp>> {
        if let Some(a) = self.cache.get(id) {
            return Some(a.clone());
        }
        let dir = self.dir.as_ref()?;
        let s = std::fs::read_to_string(dir.join("apps").join(format!("{}.json", safe(id)))).ok()?;
        let a: KnxApp = serde_json::from_str(&s).ok()?;
        let a = Arc::new(a);
        self.cache.insert(id.to_string(), a.clone());
        Some(a)
    }

    /// Add imported products and applications. Returns (new products, updated products).
    pub fn add(&mut self, products: Vec<Product>, apps: Vec<KnxApp>) -> Result<(usize, usize), String> {
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir.join("apps")).map_err(|e| format!("Could not create the library folder: {e}"))?;
            for a in &apps {
                let s = serde_json::to_string(a).map_err(|e| e.to_string())?;
                std::fs::write(dir.join("apps").join(format!("{}.json", safe(&a.id))), s).map_err(|e| format!("Could not save {}: {e}", a.name))?;
            }
        }
        for a in apps {
            self.cache.insert(a.id.clone(), Arc::new(a));
        }
        let (mut new, mut upd) = (0, 0);
        for p in products {
            match self.products.iter_mut().find(|x| x.manufacturer == p.manufacturer && x.order_no == p.order_no && x.app_id == p.app_id && x.name == p.name) {
                Some(x) => {
                    *x = p;
                    upd += 1;
                }
                None => {
                    self.products.push(p);
                    new += 1;
                }
            }
        }
        self.save()?;
        Ok((new, upd))
    }

    /// Remember device type data (from the master data of an imported file).
    pub fn add_masks(&mut self, masks: Vec<crate::knxload::MaskInfo>) {
        if masks.is_empty() {
            return;
        }
        for m in masks {
            self.masks.insert(m.id.clone(), m);
        }
        if let Some(dir) = &self.dir {
            let _ = std::fs::create_dir_all(dir);
            if let Ok(s) = serde_json::to_string(&self.masks) {
                let _ = std::fs::write(dir.join("masks.json"), s);
            }
        }
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, i: usize) -> Result<(), String> {
        if i < self.products.len() {
            self.products.remove(i);
        }
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let s = serde_json::to_string_pretty(&self.products).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("catalog.json"), s).map_err(|e| format!("Could not save the product library: {e}"))
    }
}
