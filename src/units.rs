//! Units of measure: dimensional analysis for physical quantities.
//!
//! A quantity stores its value in SI base units plus a dimension vector over
//! (m, kg, s, A, K, mol, cd, rad). The unit it was written in is remembered for display.

use std::fmt;
use std::rc::Rc;

pub const BASE: [&str; 8] = ["m", "kg", "s", "A", "K", "mol", "cd", "rad"];

pub type Dim = [i8; 8];
pub const DIMLESS: Dim = [0; 8];

#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// Multiply a value in this unit by `scale` to get SI.
    pub scale: f64,
    pub dim: Dim,
    /// Canonical spelling, e.g. "m/s^2".
    pub name: String,
}

const M: Dim = [1, 0, 0, 0, 0, 0, 0, 0];
const KG: Dim = [0, 1, 0, 0, 0, 0, 0, 0];
const S: Dim = [0, 0, 1, 0, 0, 0, 0, 0];
const A: Dim = [0, 0, 0, 1, 0, 0, 0, 0];
const K: Dim = [0, 0, 0, 0, 1, 0, 0, 0];
const MOL: Dim = [0, 0, 0, 0, 0, 1, 0, 0];
const CD: Dim = [0, 0, 0, 0, 0, 0, 1, 0];
const RAD: Dim = [0, 0, 0, 0, 0, 0, 0, 1];
const HZ: Dim = [0, 0, -1, 0, 0, 0, 0, 0];
const N: Dim = [1, 1, -2, 0, 0, 0, 0, 0];
const J: Dim = [2, 1, -2, 0, 0, 0, 0, 0];
const W: Dim = [2, 1, -3, 0, 0, 0, 0, 0];
const PA: Dim = [-1, 1, -2, 0, 0, 0, 0, 0];
const V: Dim = [2, 1, -3, -1, 0, 0, 0, 0];
const OHM: Dim = [2, 1, -3, -2, 0, 0, 0, 0];
const COUL: Dim = [0, 0, 1, 1, 0, 0, 0, 0];

/// (symbol, scale-to-SI, dimension)
const ATOMS: &[(&str, f64, Dim)] = &[
    // length
    ("m", 1.0, M),
    ("km", 1e3, M),
    ("cm", 1e-2, M),
    ("mm", 1e-3, M),
    ("um", 1e-6, M),
    ("nm", 1e-9, M),
    ("in", 0.0254, M),
    ("ft", 0.3048, M),
    ("yd", 0.9144, M),
    ("mi", 1609.344, M),
    // mass
    ("kg", 1.0, KG),
    ("g", 1e-3, KG),
    ("mg", 1e-6, KG),
    ("t", 1e3, KG),
    ("lb", 0.453_592_37, KG),
    ("oz", 0.028_349_523_125, KG),
    // time
    ("s", 1.0, S),
    ("ms", 1e-3, S),
    ("us", 1e-6, S),
    ("ns", 1e-9, S),
    ("min", 60.0, S),
    ("h", 3600.0, S),
    // electrical
    ("A", 1.0, A),
    ("mA", 1e-3, A),
    ("uA", 1e-6, A),
    ("V", 1.0, V),
    ("mV", 1e-3, V),
    ("kV", 1e3, V),
    ("ohm", 1.0, OHM),
    ("kohm", 1e3, OHM),
    ("Mohm", 1e6, OHM),
    ("C", 1.0, COUL),
    ("mAh", 3.6, COUL),
    ("Ah", 3600.0, COUL),
    // other base
    ("K", 1.0, K),
    ("mol", 1.0, MOL),
    ("cd", 1.0, CD),
    // angle
    ("rad", 1.0, RAD),
    ("deg", std::f64::consts::PI / 180.0, RAD),
    ("rev", std::f64::consts::TAU, RAD),
    // derived
    ("Hz", 1.0, HZ),
    ("kHz", 1e3, HZ),
    ("MHz", 1e6, HZ),
    (
        "rpm",
        std::f64::consts::TAU / 60.0,
        [0, 0, -1, 0, 0, 0, 0, 1],
    ),
    ("N", 1.0, N),
    ("kN", 1e3, N),
    ("J", 1.0, J),
    ("kJ", 1e3, J),
    ("Wh", 3600.0, J),
    ("W", 1.0, W),
    ("mW", 1e-3, W),
    ("kW", 1e3, W),
    ("Pa", 1.0, PA),
    ("kPa", 1e3, PA),
    ("bar", 1e5, PA),
    ("psi", 6_894.757_293_168, PA),
    ("kph", 1.0 / 3.6, [1, 0, -1, 0, 0, 0, 0, 0]),
    ("mph", 0.447_04, [1, 0, -1, 0, 0, 0, 0, 0]),
    ("gee", 9.806_65, [1, 0, -2, 0, 0, 0, 0, 0]),
];

/// Names used when printing a dimension that has no remembered unit.
const NAMED: &[(&str, Dim)] = &[
    ("N", N),
    ("J", J),
    ("W", W),
    ("Pa", PA),
    ("V", V),
    ("ohm", OHM),
    ("C", COUL),
    ("Hz", HZ),
];

fn atom(sym: &str) -> Option<(f64, Dim)> {
    ATOMS
        .iter()
        .find(|(s, _, _)| *s == sym)
        .map(|(_, k, d)| (*k, *d))
}

pub fn is_unit_symbol(s: &str) -> bool {
    atom(s).is_some()
}

fn add(a: Dim, b: Dim, sign: i8) -> Dim {
    let mut out = a;
    for i in 0..8 {
        out[i] += sign * b[i];
    }
    out
}

pub fn mul_dim(a: Dim, b: Dim) -> Dim {
    add(a, b, 1)
}
pub fn div_dim(a: Dim, b: Dim) -> Dim {
    add(a, b, -1)
}
pub fn pow_dim(a: Dim, n: i32) -> Dim {
    let mut out = a;
    for x in out.iter_mut() {
        *x = (*x as i32 * n) as i8;
    }
    out
}

/// Parses unit expressions like `m/s^2`, `kg*m/s^2`, `N.m`, `1/s`, `rad/s`, `m^-1`.
pub fn parse(src: &str) -> Result<Unit, String> {
    let s: String = src.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err("empty unit".into());
    }
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut scale = 1.0;
    let mut dim = DIMLESS;
    let mut sign = 1i8; // 1 = numerator, -1 = after '/'
    let mut first = true;
    while i < bytes.len() {
        if !first {
            match bytes[i] {
                '*' | '.' | '·' => {} // stays on the current side of '/'
                '/' => sign = -1,
                c => return Err(format!("unexpected '{c}' in unit '{src}'")),
            }
            i += 1;
        }
        first = false;
        let start = i;
        while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
            i += 1;
        }
        let sym: String = bytes[start..i].iter().collect();
        let mut exp: i32 = 1;
        if i < bytes.len() && bytes[i] == '^' {
            i += 1;
            let es = i;
            if i < bytes.len() && bytes[i] == '-' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let e: String = bytes[es..i].iter().collect();
            exp = e
                .parse()
                .map_err(|_| format!("bad exponent in unit '{src}'"))?;
        }
        let (k, d) = if sym == "1" {
            (1.0, DIMLESS)
        } else {
            atom(&sym).ok_or_else(|| format!("unknown unit '{sym}'"))?
        };
        let e = exp * sign as i32;
        scale *= k.powi(e);
        dim = add(dim, pow_dim(d, e), 1);
    }
    Ok(Unit {
        scale,
        dim,
        name: s,
    })
}

/// Formats a dimension as base units, e.g. `kg*m/s^2`, or a named derived unit.
pub fn dim_name(d: &Dim) -> String {
    if let Some((n, _)) = NAMED.iter().find(|(_, x)| x == d) {
        return n.to_string();
    }
    let mut num = Vec::new();
    let mut den = Vec::new();
    for (i, e) in d.iter().enumerate() {
        let part = |e: i8| {
            if e == 1 {
                BASE[i].to_string()
            } else {
                format!("{}^{e}", BASE[i])
            }
        };
        if *e > 0 {
            num.push(part(*e));
        } else if *e < 0 {
            den.push(part(-*e));
        }
    }
    let n = if num.is_empty() {
        "1".to_string()
    } else {
        num.join("*")
    };
    if den.is_empty() {
        n
    } else {
        format!("{n}/{}", den.join("/"))
    }
}

/// Human description of a dimension (for error messages).
pub fn describe(d: &Dim) -> String {
    const KINDS: &[(&str, Dim)] = &[
        ("length", M),
        ("mass", KG),
        ("time", S),
        ("current", A),
        ("temperature", K),
        ("angle", RAD),
        ("frequency", HZ),
        ("force", N),
        ("energy", J),
        ("power", W),
        ("pressure", PA),
        ("voltage", V),
        ("resistance", OHM),
        ("charge", COUL),
        ("speed", [1, 0, -1, 0, 0, 0, 0, 0]),
        ("acceleration", [1, 0, -2, 0, 0, 0, 0, 0]),
        ("angular velocity", [0, 0, -1, 0, 0, 0, 0, 1]),
    ];
    if *d == DIMLESS {
        return "dimensionless".into();
    }
    match KINDS.iter().find(|(_, x)| x == d) {
        Some((k, _)) => format!("{k} [{}]", dim_name(d)),
        None => format!("[{}]", dim_name(d)),
    }
}

/// A physical quantity.
#[derive(Clone, Debug)]
pub struct Quantity {
    pub si: f64,
    pub dim: Dim,
    /// Unit used for display (keeps `90 deg` printing as degrees).
    pub unit: Option<Rc<Unit>>,
}

impl Quantity {
    pub fn new(value: f64, unit: Rc<Unit>) -> Self {
        Quantity {
            si: value * unit.scale,
            dim: unit.dim,
            unit: Some(unit),
        }
    }
    pub fn si(si: f64, dim: Dim) -> Self {
        Quantity {
            si,
            dim,
            unit: None,
        }
    }
    /// Value expressed in the display unit.
    pub fn display_value(&self) -> f64 {
        match &self.unit {
            Some(u) => self.si / u.scale,
            None => self.si,
        }
    }
    pub fn unit_name(&self) -> String {
        match &self.unit {
            Some(u) => u.name.clone(),
            None => dim_name(&self.dim),
        }
    }
    pub fn to(&self, u: &str) -> Result<Quantity, String> {
        let unit = parse(u)?;
        if unit.dim != self.dim {
            return Err(format!(
                "cannot convert {} to '{u}' ({})",
                describe(&self.dim),
                describe(&unit.dim)
            ));
        }
        Ok(Quantity {
            si: self.si,
            dim: self.dim,
            unit: Some(Rc::new(unit)),
        })
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let v = self.display_value();
        // hide binary floating-point noise from unit conversion (29.999999999999996 -> 30)
        let r = if v != 0.0 && v.is_finite() {
            let digits = 12 - v.abs().log10().ceil() as i32;
            let p = 10f64.powi(digits.clamp(0, 15));
            (v * p).round() / p
        } else {
            v
        };
        let shown = if r.fract() == 0.0 && r.abs() < 1e15 {
            format!("{}", r as i64)
        } else {
            format!("{r}")
        };
        write!(f, "{shown} {}", self.unit_name())
    }
}

/// All unit symbols (for completion in tooling).
pub fn unit_symbols() -> Vec<&'static str> {
    ATOMS.iter().map(|(s, _, _)| *s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_compound() {
        let u = parse("kg*m/s^2").unwrap();
        assert_eq!(u.dim, N);
        let u = parse("km/h").unwrap();
        assert!((u.scale - 1.0 / 3.6).abs() < 1e-12);
        assert_eq!(parse("1/s").unwrap().dim, HZ);
        assert_eq!(parse("m^-1").unwrap().dim, [-1, 0, 0, 0, 0, 0, 0, 0]);
        assert!(parse("furlong").is_err());
        assert_eq!(dim_name(&[1, 0, -2, 0, 0, 0, 0, 0]), "m/s^2");
    }
}
