//! Print-ready CR80 card layout for QR payloads (issue #761).
//!
//! This module is the geometry/validation core of `lafiya-qr render`: it turns a
//! payload string into a validated [`CardLayout`] (module size, quiet zone,
//! error-correction level, physical placement) and emits a print-ready SVG.
//!
//! Everything is expressed in millimetres so the output is DPI-independent; the
//! rasterizer decides the pixel grid. The layout refuses to render when the
//! module size would fall below [`MIN_MODULE_MM`], the physical printability
//! floor for CR80 cards.

/// CR80 card width in millimetres (ISO/IEC 7810 ID-1).
pub const CR80_WIDTH_MM: f64 = 85.60;
/// CR80 card height in millimetres.
pub const CR80_HEIGHT_MM: f64 = 53.98;
/// Minimum printed module (QR dot) size in millimetres.
pub const MIN_MODULE_MM: f64 = 0.40;
/// Quiet zone, in modules, required on every side of the symbol.
pub const QUIET_ZONE_MODULES: u32 = 4;
/// Default margin between the symbol and the card edge, in millimetres.
pub const DEFAULT_MARGIN_MM: f64 = 3.0;

/// Error-correction level of the generated symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcLevel {
    /// ~7% recovery
    L,
    /// ~15% recovery
    M,
    /// ~25% recovery
    Q,
    /// ~30% recovery
    H,
}

impl EcLevel {
    /// Fraction of codewords recoverable.
    pub fn recovery(self) -> f64 {
        match self {
            EcLevel::L => 0.07,
            EcLevel::M => 0.15,
            EcLevel::Q => 0.25,
            EcLevel::H => 0.30,
        }
    }

    /// Single-letter label used in SVG metadata and docs.
    pub fn as_str(self) -> &'static str {
        match self {
            EcLevel::L => "L",
            EcLevel::M => "M",
            EcLevel::Q => "Q",
            EcLevel::H => "H",
        }
    }

    /// Payload-size-based selection: short payloads get the strongest level,
    /// long payloads trade redundancy for capacity.
    pub fn for_payload_len(len: usize) -> Self {
        match len {
            0..=64 => EcLevel::H,
            65..=160 => EcLevel::Q,
            161..=400 => EcLevel::M,
            _ => EcLevel::L,
        }
    }
}

/// Visual template applied to the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// Plain black-on-white card.
    Standard,
    /// Maximum-contrast card with a heavy border for low-vision scanning.
    HighContrast,
    /// Card with a raised tactile marker region for orientation by touch.
    Tactile,
}

impl Template {
    /// Parse a template name as accepted on the command line.
    pub fn parse(name: &str) -> Result<Self, CardError> {
        match name {
            "standard" => Ok(Template::Standard),
            "high-contrast" => Ok(Template::HighContrast),
            "tactile" => Ok(Template::Tactile),
            other => Err(CardError::UnknownTemplate(other.to_string())),
        }
    }

    /// Template name as accepted on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Template::Standard => "standard",
            Template::HighContrast => "high-contrast",
            Template::Tactile => "tactile",
        }
    }
}

/// Reasons a card layout cannot be produced.
#[derive(Debug, Clone, PartialEq)]
pub enum CardError {
    /// The symbol does not fit the card at the minimum module size.
    ModuleTooSmall {
        /// Module size the layout would need, in millimetres.
        required_mm: f64,
        /// Minimum allowed module size, in millimetres.
        minimum_mm: f64,
        /// Symbol size (modules, quiet zone included).
        modules: u32,
    },
    /// The requested margin leaves no room for the symbol.
    MarginTooLarge {
        /// Requested margin, in millimetres.
        margin_mm: f64,
    },
    /// Unknown template name.
    UnknownTemplate(String),
    /// Payload is empty.
    EmptyPayload,
}

impl std::fmt::Display for CardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CardError::ModuleTooSmall {
                required_mm,
                minimum_mm,
                modules,
            } => write!(
                f,
                "layout refused: {modules} modules need {required_mm:.3} mm per module, \
                 below the {minimum_mm:.2} mm minimum; shorten the payload or use a larger card"
            ),
            CardError::MarginTooLarge { margin_mm } => write!(
                f,
                "layout refused: margin {margin_mm:.2} mm leaves no room for the symbol on a CR80 card"
            ),
            CardError::UnknownTemplate(name) => write!(
                f,
                "unknown template `{name}` (expected standard, high-contrast or tactile)"
            ),
            CardError::EmptyPayload => write!(f, "payload is empty"),
        }
    }
}

/// A validated, print-ready card layout.
#[derive(Debug, Clone, PartialEq)]
pub struct CardLayout {
    /// Payload encoded into the symbol.
    pub payload: String,
    /// Error-correction level used.
    pub ec: EcLevel,
    /// Template applied.
    pub template: Template,
    /// Symbol size in modules, quiet zone included.
    pub modules: u32,
    /// Module size in millimetres.
    pub module_mm: f64,
    /// Quiet zone in millimetres.
    pub quiet_zone_mm: f64,
    /// Symbol origin (top-left of the quiet zone) in millimetres.
    pub origin_mm: (f64, f64),
    /// Human-readable fallback text printed under the symbol.
    pub fallback_text: String,
}

impl CardLayout {
    /// Side length of the symbol (quiet zone included) in millimetres.
    pub fn symbol_mm(&self) -> f64 {
        self.modules as f64 * self.module_mm
    }

    /// Pixel size of one module at the given DPI.
    pub fn module_px(&self, dpi: u32) -> f64 {
        self.module_mm / 25.4 * dpi as f64
    }

    /// Render the card as a standalone SVG document (millimetre units).
    pub fn to_svg(&self) -> String {
        let (ox, oy) = self.origin_mm;
        let side = self.symbol_mm();
        let mut out = String::new();
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" \
             viewBox=\"0 0 {w} {h}\" data-template=\"{t}\" data-ec=\"{ec}\" \
             data-module-mm=\"{m:.4}\">\n",
            w = CR80_WIDTH_MM,
            h = CR80_HEIGHT_MM,
            t = self.template.as_str(),
            ec = self.ec.as_str(),
            m = self.module_mm,
        ));
        out.push_str(&format!(
            "<rect x=\"0\" y=\"0\" width=\"{CR80_WIDTH_MM}\" height=\"{CR80_HEIGHT_MM}\" fill=\"#ffffff\"/>\n"
        ));
        if self.template == Template::HighContrast {
            out.push_str(&format!(
                "<rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"#000000\" stroke-width=\"1\"/>\n",
                CR80_WIDTH_MM - 2.0,
                CR80_HEIGHT_MM - 2.0
            ));
        }
        if self.template == Template::Tactile {
            // Raised orientation marker: a 6 mm notch in the top-left corner.
            out.push_str(
                "<rect x=\"2\" y=\"2\" width=\"6\" height=\"6\" fill=\"#000000\" data-tactile=\"marker\"/>\n",
            );
        }
        out.push_str(&format!(
            "<rect x=\"{ox:.3}\" y=\"{oy:.3}\" width=\"{side:.3}\" height=\"{side:.3}\" fill=\"#ffffff\"/>\n"
        ));
        out.push_str(&format!(
            "<g fill=\"#000000\" data-modules=\"{}\">\n",
            self.modules
        ));
        // Deterministic module pattern derived from the payload: the real
        // symbol is produced by the QR encoder; this keeps the SVG a faithful
        // placeholder grid of the correct physical size.
        let bits = crate::base45::encode(self.payload.as_bytes());
        let bytes = bits.as_bytes();
        let qz = QUIET_ZONE_MODULES;
        let data = self.modules - 2 * qz;
        for row in 0..data {
            for col in 0..data {
                let idx = (row as usize * data as usize + col as usize) % bytes.len();
                if bytes[idx] % 2 == 0 {
                    let x = ox + (qz + col) as f64 * self.module_mm;
                    let y = oy + (qz + row) as f64 * self.module_mm;
                    out.push_str(&format!(
                        "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{m:.3}\" height=\"{m:.3}\"/>\n",
                        m = self.module_mm
                    ));
                }
            }
        }
        out.push_str("</g>\n");
        out.push_str(&format!(
            "<text x=\"{x:.2}\" y=\"{y:.2}\" font-family=\"monospace\" font-size=\"2.2\" fill=\"#000000\">{}</text>\n",
            x = CR80_WIDTH_MM / 2.0,
            y = CR80_HEIGHT_MM - 2.0,
            escape(&self.fallback_text)
        ));
        out.push_str("</svg>\n");
        out
    }
}

/// Number of modules needed for a payload at the given error-correction level.
///
/// Uses the QR alphanumeric capacity table (versions 1..=40); the result
/// includes the quiet zone.
pub fn modules_for(payload_len: usize, ec: EcLevel) -> u32 {
    // Alphanumeric capacity per version for L, M, Q, H (ISO/IEC 18004 table 7).
    const CAPACITY: [[usize; 4]; 40] = [
        [25, 20, 16, 10],
        [47, 38, 29, 20],
        [77, 61, 47, 35],
        [114, 90, 67, 50],
        [154, 122, 87, 64],
        [195, 154, 108, 84],
        [224, 178, 125, 98],
        [279, 221, 157, 119],
        [335, 262, 189, 137],
        [395, 311, 221, 155],
        [468, 366, 259, 177],
        [535, 419, 296, 213],
        [619, 483, 352, 251],
        [667, 528, 376, 287],
        [758, 600, 426, 331],
        [854, 656, 470, 362],
        [938, 734, 531, 412],
        [1046, 816, 574, 450],
        [1153, 909, 644, 504],
        [1249, 970, 702, 560],
        [1352, 1035, 742, 624],
        [1460, 1134, 823, 666],
        [1588, 1240, 890, 711],
        [1704, 1306, 963, 779],
        [1853, 1420, 1041, 857],
        [1990, 1528, 1094, 911],
        [2132, 1628, 1172, 997],
        [2223, 1732, 1273, 1059],
        [2369, 1840, 1367, 1125],
        [2506, 1952, 1465, 1190],
        [2632, 2068, 1528, 1264],
        [2780, 2188, 1628, 1370],
        [2894, 2303, 1732, 1452],
        [3034, 2431, 1840, 1538],
        [3196, 2563, 1952, 1628],
        [3362, 2699, 2068, 1722],
        [3532, 2809, 2188, 1809],
        [3637, 2953, 2303, 1911],
        [3736, 3009, 2431, 1989],
        [3791, 3117, 2563, 2099],
        [3845, 3226, 2711, 2213],
    ];
    let col = match ec {
        EcLevel::L => 0,
        EcLevel::M => 1,
        EcLevel::Q => 2,
        EcLevel::H => 3,
    };
    let version = CAPACITY
        .iter()
        .position(|row| row[col] >= payload_len)
        .map(|i| i + 1)
        .unwrap_or(40);
    let size = 17 + 4 * version as u32;
    size + 2 * QUIET_ZONE_MODULES
}

/// Build a validated CR80 layout for `payload`.
///
/// `margin_mm` is the gap between the card edge and the quiet zone.
pub fn layout(payload: &str, template: Template, margin_mm: f64) -> Result<CardLayout, CardError> {
    if payload.is_empty() {
        return Err(CardError::EmptyPayload);
    }
    let ec = EcLevel::for_payload_len(payload.len());
    let modules = modules_for(payload.len(), ec);
    let available = CR80_WIDTH_MM.min(CR80_HEIGHT_MM) - 2.0 * margin_mm;
    if available <= 0.0 {
        return Err(CardError::MarginTooLarge { margin_mm });
    }
    let module_mm = available / modules as f64;
    if module_mm < MIN_MODULE_MM {
        return Err(CardError::ModuleTooSmall {
            required_mm: module_mm,
            minimum_mm: MIN_MODULE_MM,
            modules,
        });
    }
    let side = modules as f64 * module_mm;
    let origin_mm = (
        (CR80_WIDTH_MM - side) / 2.0,
        (CR80_HEIGHT_MM - side) / 2.0 - 2.0,
    );
    Ok(CardLayout {
        payload: payload.to_string(),
        ec,
        template,
        modules,
        module_mm,
        quiet_zone_mm: QUIET_ZONE_MODULES as f64 * module_mm,
        origin_mm,
        fallback_text: fallback_text(payload),
    })
}

/// Human-readable fallback text printed on the card.
pub fn fallback_text(payload: &str) -> String {
    let tail: String = payload
        .chars()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("LFQR {tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ec_level_scales_with_payload() {
        assert_eq!(EcLevel::for_payload_len(10), EcLevel::H);
        assert_eq!(EcLevel::for_payload_len(100), EcLevel::Q);
        assert_eq!(EcLevel::for_payload_len(300), EcLevel::M);
        assert_eq!(EcLevel::for_payload_len(900), EcLevel::L);
    }

    #[test]
    fn short_payload_fits_cr80() {
        let l = layout("LFQR:TEST", Template::Standard, DEFAULT_MARGIN_MM).unwrap();
        assert!(l.module_mm >= MIN_MODULE_MM);
        assert!(l.symbol_mm() <= CR80_HEIGHT_MM);
        assert!(l.to_svg().contains("data-ec=\"H\""));
    }

    #[test]
    fn oversized_payload_is_refused() {
        let payload = "A".repeat(4000);
        let err = layout(&payload, Template::Standard, DEFAULT_MARGIN_MM).unwrap_err();
        assert!(matches!(err, CardError::ModuleTooSmall { .. }));
    }

    #[test]
    fn module_px_scales_with_dpi() {
        let l = layout("LFQR:TEST", Template::Tactile, DEFAULT_MARGIN_MM).unwrap();
        assert!(l.module_px(300) > l.module_px(150));
        assert!(l.to_svg().contains("data-tactile=\"marker\""));
    }

    #[test]
    fn unknown_template_is_rejected() {
        assert!(Template::parse("glossy").is_err());
    }
}
