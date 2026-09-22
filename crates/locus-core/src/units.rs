use serde::{Deserialize, Serialize};

/// Linear unit a source file's coordinates are expressed in.
/// Internal geometry is always meters; this only describes source data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearUnit {
    Meter,
    Centimeter,
    Millimeter,
    Foot,
    UsSurveyFoot,
    Inch,
}

impl LinearUnit {
    /// Meters in one of this unit. All values are exact by definition.
    pub fn meters(self) -> f64 {
        match self {
            Self::Meter => 1.0,
            Self::Centimeter => 0.01,
            Self::Millimeter => 0.001,
            Self::Foot => 0.3048,
            Self::UsSurveyFoot => 1200.0 / 3937.0,
            Self::Inch => 0.0254,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LinearUnit::*;

    #[test]
    fn factors_match_definitions() {
        assert_eq!(Foot.meters(), 0.3048);
        assert!((Inch.meters() * 12.0 - Foot.meters()).abs() < 1e-15);
        // US survey foot is 1200/3937 m, about 2 ppm longer than the international foot.
        assert!((UsSurveyFoot.meters() - 0.304_800_609_601_219_2).abs() < 1e-15);
        assert!((UsSurveyFoot.meters() / Foot.meters() - 1.0 - 2.0e-6).abs() < 1e-8);
    }
}
