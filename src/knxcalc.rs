//! A small JavaScript-subset interpreter for parameter calculations in product files
//! (ParameterCalculation / LRTransformation). Numbers only; supports var/let/const,
//! assignments (= += -= *= /= %= |= &= ^= <<= >>=), if/else, while, for, switch/case,
//! break/continue, the usual operators, ternary, Math.* and simple functions.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Str(String),
    P(&'static str),
}

const PUNCT: &[&str] = &[
    ">>>=", "===", "!==", ">>>", "<<=", ">>=", "**", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "<<", ">>", "++", "--", "+", "-", "*", "/", "%", "<", ">",
    "=", "!", "~", "&", "|", "^", "?", ":", "(", ")", "{", "}", ";", ",", ".", "[", "]",
];

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).map(|d| d.is_ascii_digit()).unwrap_or(false)) {
            let s = i;
            if ch == '0' && matches!(c.get(i + 1), Some('x') | Some('X')) {
                i += 2;
                while i < c.len() && c[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let h: String = c[s + 2..i].iter().collect();
                out.push(Tok::Num(i64::from_str_radix(&h, 16).map_err(|e| e.to_string())? as f64));
                continue;
            }
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == 'e' || c[i] == 'E') {
                i += 1;
            }
            let n: String = c[s..i].iter().collect();
            out.push(Tok::Num(n.parse().map_err(|_| format!("bad number {n}"))?));
        } else if ch.is_alphabetic() || ch == '_' || ch == '$' {
            let s = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                i += 1;
            }
            out.push(Tok::Id(c[s..i].iter().collect()));
        } else if ch == '"' || ch == '\'' {
            let q = ch;
            i += 1;
            let s = i;
            while i < c.len() && c[i] != q {
                i += 1;
            }
            out.push(Tok::Str(c[s..i.min(c.len())].iter().collect()));
            i += 1;
        } else {
            let rest: String = c[i..(i + 4).min(c.len())].iter().collect();
            let p = PUNCT.iter().find(|p| rest.starts_with(**p)).ok_or_else(|| format!("unexpected character '{ch}'"))?;
            out.push(Tok::P(p));
            i += p.chars().count();
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Var(String),
    Un(&'static str, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Assign(&'static str, String, Box<Expr>),
    IncDec(String, f64, bool),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Arr(Vec<Expr>),
    Index(String, Box<Expr>),
    SetIndex(String, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
enum Stmt {
    Expr(Expr),
    Decl(Vec<(String, Option<Expr>)>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Block(Vec<Stmt>),
    Break,
    Continue,
    Return(Option<Expr>),
    Func(String, Vec<String>, Vec<Stmt>),
    Empty,
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(x)) if *x == p)
    }
    fn is_id(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(x)) if x == s)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, p: &str) -> Result<(), String> {
        if self.eat(p) { Ok(()) } else { Err(format!("expected '{p}'")) }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Id(s)) => {
                self.i += 1;
                Ok(s)
            }
            _ => Err("expected a name".into()),
        }
    }

    fn program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut v = vec![];
        while self.peek().is_some() {
            v.push(self.stmt()?);
        }
        Ok(v)
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat("{") {
            let mut v = vec![];
            while !self.eat("}") {
                if self.peek().is_none() {
                    return Err("missing '}'".into());
                }
                v.push(self.stmt()?);
            }
            return Ok(Stmt::Block(v));
        }
        if self.is_id("var") || self.is_id("let") || self.is_id("const") {
            self.i += 1;
            let d = self.decls()?;
            self.eat(";");
            return Ok(d);
        }
        if self.is_id("if") {
            self.i += 1;
            self.expect("(")?;
            let c = self.expr()?;
            self.expect(")")?;
            let a = self.stmt()?;
            let b = if self.is_id("else") {
                self.i += 1;
                Some(Box::new(self.stmt()?))
            } else {
                None
            };
            return Ok(Stmt::If(c, Box::new(a), b));
        }
        if self.is_id("while") {
            self.i += 1;
            self.expect("(")?;
            let c = self.expr()?;
            self.expect(")")?;
            return Ok(Stmt::While(c, Box::new(self.stmt()?)));
        }
        if self.is_id("for") {
            self.i += 1;
            self.expect("(")?;
            let init = if self.eat(";") {
                None
            } else if self.is_id("var") || self.is_id("let") || self.is_id("const") {
                self.i += 1;
                let d = self.decls()?;
                self.expect(";")?;
                Some(Box::new(d))
            } else {
                let e = self.expr()?;
                self.expect(";")?;
                Some(Box::new(Stmt::Expr(e)))
            };
            let cond = if self.is(";") { None } else { Some(self.expr()?) };
            self.expect(";")?;
            let step = if self.is(")") { None } else { Some(self.expr()?) };
            self.expect(")")?;
            return Ok(Stmt::For(init, cond, step, Box::new(self.stmt()?)));
        }
        if self.is_id("switch") {
            self.i += 1;
            self.expect("(")?;
            let e = self.expr()?;
            self.expect(")")?;
            self.expect("{")?;
            let mut cases: Vec<(Option<Expr>, Vec<Stmt>)> = vec![];
            while !self.eat("}") {
                if self.is_id("case") {
                    self.i += 1;
                    let v = self.expr()?;
                    self.expect(":")?;
                    cases.push((Some(v), vec![]));
                } else if self.is_id("default") {
                    self.i += 1;
                    self.expect(":")?;
                    cases.push((None, vec![]));
                } else {
                    let s = self.stmt()?;
                    cases.last_mut().ok_or("statement outside case")?.1.push(s);
                }
            }
            return Ok(Stmt::Switch(e, cases));
        }
        if self.is_id("break") {
            self.i += 1;
            self.eat(";");
            return Ok(Stmt::Break);
        }
        if self.is_id("continue") {
            self.i += 1;
            self.eat(";");
            return Ok(Stmt::Continue);
        }
        if self.is_id("return") {
            self.i += 1;
            let e = if self.is(";") || self.is("}") { None } else { Some(self.expr()?) };
            self.eat(";");
            return Ok(Stmt::Return(e));
        }
        if self.is_id("function") {
            self.i += 1;
            let name = self.ident()?;
            self.expect("(")?;
            let mut params = vec![];
            while !self.eat(")") {
                params.push(self.ident()?);
                self.eat(",");
            }
            let Stmt::Block(body) = self.stmt()? else { return Err("function body expected".into()) };
            return Ok(Stmt::Func(name, params, body));
        }
        let e = self.expr()?;
        self.eat(";");
        Ok(Stmt::Expr(e))
    }

    fn decls(&mut self) -> Result<Stmt, String> {
        let mut v = vec![];
        loop {
            let n = self.ident()?;
            let e = if self.eat("=") { Some(self.assign()?) } else { None };
            v.push((n, e));
            if !self.eat(",") {
                break;
            }
        }
        Ok(Stmt::Decl(v))
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut e = self.assign()?;
        while self.eat(",") {
            e = self.assign()?;
        }
        Ok(e)
    }

    fn assign(&mut self) -> Result<Expr, String> {
        let lhs = self.cond()?;
        for op in ["=", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "<<=", ">>="] {
            if self.is(op) {
                self.i += 1;
                if let Expr::Index(name, idx) = lhs {
                    if op != "=" {
                        return Err("compound assignment to array elements is not supported".into());
                    }
                    let rhs = self.assign()?;
                    return Ok(Expr::SetIndex(name, idx, Box::new(rhs)));
                }
                let Expr::Var(name) = lhs else { return Err("can only assign to a name".into()) };
                let rhs = self.assign()?;
                let op: &'static str = PUNCT.iter().find(|p| **p == op).unwrap();
                return Ok(Expr::Assign(op, name, Box::new(rhs)));
            }
        }
        Ok(lhs)
    }

    fn cond(&mut self) -> Result<Expr, String> {
        let c = self.bin(0)?;
        if self.eat("?") {
            let a = self.assign()?;
            self.expect(":")?;
            let b = self.assign()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn bin(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["|"], &["^"], &["&"], &["==", "!=", "===", "!=="], &["<", ">", "<=", ">="], &["<<", ">>", ">>>"], &["+", "-"], &["*", "/", "%"], &["**"]];
        if level >= LEVELS.len() {
            return self.unary();
        }
        let mut l = self.bin(level + 1)?;
        loop {
            let Some(op) = LEVELS[level].iter().find(|op| self.is(op)).copied() else { break };
            self.i += 1;
            let r = self.bin(level + 1)?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        for op in ["!", "-", "+", "~"] {
            if self.is(op) {
                self.i += 1;
                let e = self.unary()?;
                let op: &'static str = PUNCT.iter().find(|p| **p == op).unwrap();
                return Ok(Expr::Un(op, Box::new(e)));
            }
        }
        for (op, d) in [("++", 1.0), ("--", -1.0)] {
            if self.is(op) {
                self.i += 1;
                let n = self.ident()?;
                return Ok(Expr::IncDec(n, d, true));
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let e = self.primary()?;
        if let Expr::Var(n) = &e {
            for (op, d) in [("++", 1.0), ("--", -1.0)] {
                if self.is(op) {
                    self.i += 1;
                    return Ok(Expr::IncDec(n.clone(), d, false));
                }
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Expr::Num(s.parse().unwrap_or(0.0)))
            }
            Some(Tok::Id(mut n)) => {
                self.i += 1;
                match n.as_str() {
                    "true" => return Ok(Expr::Num(1.0)),
                    "false" | "null" | "undefined" => return Ok(Expr::Num(0.0)),
                    _ => {}
                }
                while self.eat(".") {
                    n = format!("{n}.{}", self.ident()?);
                }
                if self.eat("(") {
                    let mut args = vec![];
                    while !self.eat(")") {
                        args.push(self.assign()?);
                        self.eat(",");
                        if self.peek().is_none() {
                            return Err("missing ')'".into());
                        }
                    }
                    return Ok(Expr::Call(n, args));
                }
                if self.eat("[") {
                    let idx = self.expr()?;
                    self.expect("]")?;
                    return Ok(Expr::Index(n, Box::new(idx)));
                }
                Ok(Expr::Var(n))
            }
            Some(Tok::P("(")) => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::P("[")) => {
                self.i += 1;
                let mut v = vec![];
                while !self.eat("]") {
                    v.push(self.assign()?);
                    self.eat(",");
                    if self.peek().is_none() {
                        return Err("missing ']'".into());
                    }
                }
                Ok(Expr::Arr(v))
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return(f64),
}

pub struct Interp {
    pub vars: HashMap<String, f64>,
    arrays: HashMap<String, Vec<f64>>,
    funcs: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    steps: u64,
}

fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}
fn b(v: bool) -> f64 {
    if v { 1.0 } else { 0.0 }
}

impl Interp {
    fn tick(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > 200_000 {
            return Err("calculation takes too long".into());
        }
        Ok(())
    }

    fn eval(&mut self, e: &Expr) -> Result<f64, String> {
        self.tick()?;
        Ok(match e {
            Expr::Num(n) => *n,
            Expr::Var(n) => match n.strip_suffix(".length").and_then(|b| self.arrays.get(b)) {
                Some(a) => a.len() as f64,
                None => *self.vars.get(n).unwrap_or(&0.0),
            },
            Expr::Arr(_) => return Err("array used as a number".into()),
            Expr::Index(n, i) => {
                let i = self.eval(i)?;
                let a = self.arrays.get(n).ok_or_else(|| format!("{n} is not an array"))?;
                a.get(i as usize).copied().unwrap_or(f64::NAN)
            }
            Expr::SetIndex(n, i, x) => {
                let i = self.eval(i)? as usize;
                let v = self.eval(x)?;
                let a = self.arrays.entry(n.clone()).or_default();
                if a.len() <= i {
                    a.resize(i + 1, 0.0);
                }
                a[i] = v;
                v
            }
            Expr::Un(op, x) => {
                let v = self.eval(x)?;
                match *op {
                    "!" => b(!truthy(v)),
                    "-" => -v,
                    "~" => !(v as i64 as i32) as f64,
                    _ => v,
                }
            }
            Expr::Bin(op, l, r) => {
                if *op == "&&" {
                    let a = self.eval(l)?;
                    return Ok(if truthy(a) { self.eval(r)? } else { a });
                }
                if *op == "||" {
                    let a = self.eval(l)?;
                    return Ok(if truthy(a) { a } else { self.eval(r)? });
                }
                let (a, c) = (self.eval(l)?, self.eval(r)?);
                let (ia, ic) = (a as i64 as i32, c as i64 as i32);
                match *op {
                    "+" => a + c,
                    "-" => a - c,
                    "*" => a * c,
                    "/" => a / c,
                    "%" => a % c,
                    "**" => a.powf(c),
                    "==" | "===" => b(a == c),
                    "!=" | "!==" => b(a != c),
                    "<" => b(a < c),
                    ">" => b(a > c),
                    "<=" => b(a <= c),
                    ">=" => b(a >= c),
                    "&" => (ia & ic) as f64,
                    "|" => (ia | ic) as f64,
                    "^" => (ia ^ ic) as f64,
                    "<<" => (ia.wrapping_shl(ic as u32 & 31)) as f64,
                    ">>" => (ia >> (ic as u32 & 31)) as f64,
                    ">>>" => ((ia as u32) >> (ic as u32 & 31)) as f64,
                    _ => return Err(format!("operator {op}")),
                }
            }
            Expr::Assign(op, n, x) => {
                if let Expr::Arr(items) = &**x {
                    let vals: Vec<f64> = items.iter().map(|e| self.eval(e)).collect::<Result<_, _>>()?;
                    self.arrays.insert(n.clone(), vals);
                    return Ok(0.0);
                }
                let v = self.eval(x)?;
                let cur = *self.vars.get(n).unwrap_or(&0.0);
                let nv = match *op {
                    "=" => v,
                    "+=" => cur + v,
                    "-=" => cur - v,
                    "*=" => cur * v,
                    "/=" => cur / v,
                    "%=" => cur % v,
                    "|=" => ((cur as i64 as i32) | (v as i64 as i32)) as f64,
                    "&=" => ((cur as i64 as i32) & (v as i64 as i32)) as f64,
                    "^=" => ((cur as i64 as i32) ^ (v as i64 as i32)) as f64,
                    "<<=" => ((cur as i64 as i32).wrapping_shl(v as u32 & 31)) as f64,
                    ">>=" => ((cur as i64 as i32) >> (v as u32 & 31)) as f64,
                    _ => v,
                };
                self.vars.insert(n.clone(), nv);
                nv
            }
            Expr::IncDec(n, d, pre) => {
                let cur = *self.vars.get(n).unwrap_or(&0.0);
                self.vars.insert(n.clone(), cur + d);
                if *pre { cur + d } else { cur }
            }
            Expr::Cond(c, a, x) => {
                if truthy(self.eval(c)?) {
                    self.eval(a)?
                } else {
                    self.eval(x)?
                }
            }
            Expr::Call(n, args) => {
                let vals: Vec<f64> = args.iter().map(|a| self.eval(a)).collect::<Result<_, _>>()?;
                let a0 = vals.first().copied().unwrap_or(0.0);
                match n.as_str() {
                    "Math.floor" => a0.floor(),
                    "Math.ceil" => a0.ceil(),
                    "Math.round" => (a0 + 0.5).floor(),
                    "Math.abs" => a0.abs(),
                    "Math.trunc" | "parseInt" => a0.trunc(),
                    "parseFloat" | "Number" => a0,
                    "Math.min" => vals.iter().copied().fold(f64::INFINITY, f64::min),
                    "Math.max" => vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    "Math.pow" => a0.powf(vals.get(1).copied().unwrap_or(1.0)),
                    "Math.sqrt" => a0.sqrt(),
                    _ => {
                        let (params, body) = self.funcs.get(n).cloned().ok_or_else(|| format!("unknown function {n}"))?;
                        let saved: Vec<(String, Option<f64>)> = params.iter().map(|p| (p.clone(), self.vars.get(p).copied())).collect();
                        for (p, v) in params.iter().zip(vals.iter().chain(std::iter::repeat(&0.0))) {
                            self.vars.insert(p.clone(), *v);
                        }
                        let mut r = 0.0;
                        for s in &body {
                            if let Flow::Return(v) = self.exec(s)? {
                                r = v;
                                break;
                            }
                        }
                        for (p, v) in saved {
                            match v {
                                Some(v) => self.vars.insert(p, v),
                                None => self.vars.remove(&p),
                            };
                        }
                        r
                    }
                }
            }
        })
    }

    fn exec(&mut self, s: &Stmt) -> Result<Flow, String> {
        self.tick()?;
        match s {
            Stmt::Expr(e) => {
                self.eval(e)?;
            }
            Stmt::Decl(v) => {
                for (n, e) in v {
                    if let Some(Expr::Arr(items)) = e {
                        let vals: Vec<f64> = items.iter().map(|e| self.eval(e)).collect::<Result<_, _>>()?;
                        self.arrays.insert(n.clone(), vals);
                        continue;
                    }
                    let x = match e {
                        Some(e) => self.eval(e)?,
                        None => 0.0,
                    };
                    self.vars.insert(n.clone(), x);
                }
            }
            Stmt::If(c, a, x) => {
                if truthy(self.eval(c)?) {
                    return self.exec(a);
                } else if let Some(x) = x {
                    return self.exec(x);
                }
            }
            Stmt::While(c, body) => {
                while truthy(self.eval(c)?) {
                    match self.exec(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Stmt::For(init, c, step, body) => {
                if let Some(i) = init {
                    self.exec(i)?;
                }
                loop {
                    if let Some(c) = c {
                        if !truthy(self.eval(c)?) {
                            break;
                        }
                    }
                    match self.exec(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    if let Some(st) = step {
                        self.eval(st)?;
                    }
                }
            }
            Stmt::Switch(e, cases) => {
                let v = self.eval(e)?;
                let mut start = None;
                for (i, (c, _)) in cases.iter().enumerate() {
                    if let Some(c) = c {
                        if self.eval(c)? == v {
                            start = Some(i);
                            break;
                        }
                    }
                }
                let start = start.or_else(|| cases.iter().position(|(c, _)| c.is_none()));
                if let Some(st) = start {
                    'outer: for (_, body) in &cases[st..] {
                        for s in body {
                            match self.exec(s)? {
                                Flow::Break => break 'outer,
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                                Flow::Continue => return Ok(Flow::Continue),
                                Flow::Normal => {}
                            }
                        }
                    }
                }
            }
            Stmt::Block(v) => {
                for s in v {
                    match self.exec(s)? {
                        Flow::Normal => {}
                        f => return Ok(f),
                    }
                }
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e)?,
                    None => 0.0,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Func(n, p, body) => {
                self.funcs.insert(n.clone(), (p.clone(), body.clone()));
            }
            Stmt::Empty => {}
        }
        Ok(Flow::Normal)
    }
}

/// Run a script with the given input variables; returns all variables afterwards.
pub fn run(script: &str, inputs: &HashMap<String, f64>) -> Result<HashMap<String, f64>, String> {
    let toks = lex(script)?;
    let prog = Parser { t: toks, i: 0 }.program()?;
    let mut it = Interp { vars: inputs.clone(), arrays: HashMap::new(), funcs: HashMap::new(), steps: 0 };
    // Functions may be declared after use
    for s in &prog {
        if let Stmt::Func(..) = s {
            it.exec(s)?;
        }
    }
    for s in &prog {
        if let Flow::Return(_) = it.exec(s)? {
            break;
        }
    }
    Ok(it.vars)
}

/// Format a calculated value like a parameter value.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ventilation_example() {
        let script = "VentilationEnabled = 0;\nif (ThermostatFunction == 1) {\n if (CommandCO == 1 && HeatingTypeVirtual == 2) { VentilationEnabled = 1; }\n if (CoolingType == 2) { VentilationEnabled = VentilationEnabled+4; }\n}\nelse {\n if (ThermostatFunction == 0) {\n  if (CommandCO == 1 && HeatingTypeVirtual == 2) { VentilationEnabled = VentilationEnabled+16; }\n }\n}";
        let mut i = HashMap::new();
        i.insert("ThermostatFunction".to_string(), 1.0);
        i.insert("CommandCO".to_string(), 1.0);
        i.insert("HeatingTypeVirtual".to_string(), 2.0);
        i.insert("CoolingType".to_string(), 2.0);
        assert_eq!(run(script, &i).unwrap()["VentilationEnabled"], 5.0);
        i.insert("ThermostatFunction".to_string(), 0.0);
        assert_eq!(run(script, &i).unwrap()["VentilationEnabled"], 16.0);
    }

    #[test]
    fn language_features() {
        let s = "var t = 0; for (var i = 0; i < 4; i++) { t += i; } switch (t) { case 6: r = 1; break; default: r = 2; }\nfunction dbl(x) { return x * 2; }\nq = dbl(t) | 1; w = t > 5 ? 10 : 20; /* c */ m = Math.max(3, 9);";
        let v = run(s, &HashMap::new()).unwrap();
        assert_eq!(v["t"], 6.0);
        assert_eq!(v["r"], 1.0);
        assert_eq!(v["q"], 13.0);
        assert_eq!(v["w"], 10.0);
        assert_eq!(v["m"], 9.0);
        let a = run("var x = [10.0, 10.5, 11.0]; y = x[2] + x.length; x[0] = 5; z = x[0];", &HashMap::new()).unwrap();
        assert_eq!(a["y"], 14.0);
        assert_eq!(a["z"], 5.0);
        // Shared-script style: function reading input.* and writing output.*
        let mut i = HashMap::new();
        i.insert("input.channel".to_string(), 2.0);
        let o = run("function F(input, output, context) { output.res = (input.channel == 2); }\nF(0, 0, 0);", &i).unwrap();
        assert_eq!(o["output.res"], 1.0);
    }
}
