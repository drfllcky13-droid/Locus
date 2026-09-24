//! Licence tiers from an offline licence file: the licence's fields and an Ed25519 signature
//! over them. The app holds only the public key; licences are signed with the private key,
//! which never enters the repository (docs/methods/licensing.md).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// 2D diagramming, hand measurements, the symbol library, reports.
    Diagram,
    /// Everything in Diagram, plus 3D scenes, point clouds, all analysis tools,
    /// photogrammetry, animation and the portable case package.
    Analyst,
    /// Everything in Analyst, plus scan registration and volumetric crush comparison.
    AnalystPlus,
}

/// What a tier unlocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Diagrams,
    PointClouds,
    Analysis,
    Photogrammetry,
    Animation,
    CasePackage,
    Registration,
    CrushVolume,
}

impl Tier {
    pub fn allows(self, f: Feature) -> bool {
        use Feature::*;
        match f {
            Diagrams => true,
            PointClouds | Analysis | Photogrammetry | Animation | CasePackage => {
                self >= Tier::Analyst
            }
            Registration | CrushVolume => self >= Tier::AnalystPlus,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tier::Diagram => "Diagram",
            Tier::Analyst => "Analyst",
            Tier::AnalystPlus => "Analyst Plus",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub licensee: String,
    pub tier: Tier,
    /// UTC date, YYYY-MM-DD.
    pub issued: String,
    /// UTC date, YYYY-MM-DD; none for a perpetual licence.
    pub expires: Option<String>,
}

/// A licence file: the licence and a hex Ed25519 signature over its canonical JSON (the
/// fields in the order above, no spaces).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseFile {
    pub license: License,
    pub signature: String,
}

fn canonical(l: &License) -> Vec<u8> {
    serde_json::to_vec(l).expect("a licence serialises")
}

pub fn sign(l: License, secret: &[u8; 32]) -> LicenseFile {
    let key = SigningKey::from_bytes(secret);
    let sig = key.sign(&canonical(&l));
    LicenseFile {
        license: l,
        signature: hex::encode(sig.to_bytes()),
    }
}

/// The public key for a private one.
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// Check a licence file's signature against `public`, and that it hasn't expired on `today`
/// (YYYY-MM-DD).
pub fn verify(text: &str, public: &[u8; 32], today: &str) -> Result<License, String> {
    let f: LicenseFile =
        serde_json::from_str(text).map_err(|e| format!("not a Locus licence file ({e})"))?;
    let key = VerifyingKey::from_bytes(public).map_err(|e| e.to_string())?;
    let bytes: [u8; 64] = hex::decode(&f.signature)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("the licence's signature is malformed")?;
    key.verify(&canonical(&f.license), &Signature::from_bytes(&bytes))
        .map_err(|_| "the licence's signature doesn't match: it was changed or isn't genuine")?;
    if let Some(e) = &f.license.expires {
        if today > e.as_str() {
            return Err(format!("the licence expired on {e}"));
        }
    }
    Ok(f.license)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lic(tier: Tier, expires: Option<&str>) -> License {
        License {
            id: "L-0001".into(),
            licensee: "County Forensic Services Unit".into(),
            tier,
            issued: "2026-09-24".into(),
            expires: expires.map(String::from),
        }
    }

    #[test]
    fn a_signed_licence_verifies_and_any_change_or_other_key_fails() {
        let secret = [7u8; 32];
        let public = public_key(&secret);
        let f = sign(lic(Tier::Analyst, Some("2027-09-24")), &secret);
        let text = serde_json::to_string_pretty(&f).unwrap();
        assert_eq!(
            verify(&text, &public, "2026-10-01").unwrap().tier,
            Tier::Analyst
        );
        // Raising the tier breaks the signature.
        let forged = text.replace("\"analyst\"", "\"analyst_plus\"");
        assert!(verify(&forged, &public, "2026-10-01")
            .unwrap_err()
            .contains("doesn't match"));
        // Another key's licence doesn't verify here.
        let other = sign(lic(Tier::AnalystPlus, None), &[9u8; 32]);
        assert!(verify(
            &serde_json::to_string(&other).unwrap(),
            &public,
            "2026-10-01"
        )
        .is_err());
        // Expired.
        assert!(verify(&text, &public, "2027-09-25")
            .unwrap_err()
            .contains("expired"));
        assert!(verify("{}", &public, "2026-10-01").is_err());
    }

    #[test]
    fn tiers_unlock_their_features() {
        assert!(Tier::Diagram.allows(Feature::Diagrams));
        assert!(!Tier::Diagram.allows(Feature::PointClouds));
        assert!(Tier::Analyst.allows(Feature::Animation));
        assert!(!Tier::Analyst.allows(Feature::Registration));
        assert!(Tier::AnalystPlus.allows(Feature::CrushVolume));
    }
}
