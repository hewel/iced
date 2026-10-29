//! CPU display conversion, before the software renderer packs opaque RGB.
//! Dolby Vision is deliberately reduced to a verified HDR10-compatible base layer.
pub(super) const BASE_FILTER: &str = "format=dolbyvision=no:enhancement-layer=no:hdr10plus=no";

#[derive(Debug, Clone, Copy)]
pub(super) enum Plan {
    Sdr,
    Pq,
    Hlg,
    DolbyVision8Base,
}
impl Plan {
    pub(super) fn classify(
        gamma: &str,
        matrix: &str,
        primaries: &str,
        source_matrix: &str,
        profile: Option<i64>,
    ) -> Result<Self, String> {
        if let Some(profile) = profile {
            if profile == 8 && gamma == "pq" && matrix == "bt.2020-ncl" && primaries == "bt.2020" {
                return Ok(Self::DolbyVision8Base);
            }
            return Err(format!(
                "Thumbnail unavailable: Dolby Vision profile {profile} without a verified PQ/BT.2020 HDR10 base layer"
            ));
        }
        if source_matrix == "dolbyvision" {
            return Err("Thumbnail unavailable: Dolby Vision profile metadata is missing".into());
        }
        match gamma {
            "pq" => Ok(Self::Pq),
            "hlg" => Ok(Self::Hlg),
            "bt.1886" | "srgb" | "gamma2.2" | "gamma2.8" => Ok(Self::Sdr),
            _ => Err(format!(
                "Thumbnail unavailable: unsupported transfer {gamma}"
            )),
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR → sRGB",
            Self::Pq => "PQ HDR → sRGB",
            Self::Hlg => "HLG HDR → sRGB",
            Self::DolbyVision8Base => "DV8 HDR10 base → sRGB (no RPU/EL)",
        }
    }
    pub(super) fn filter(self) -> String {
        // npl=100 means one linear unit is 100 cd/m². zscale carries the source
        // range/primaries and preserves display aspect through SAR. Bound the
        // floating-point work to 320x180; the renderer performs letterboxing.
        let tone = if matches!(self, Self::Sdr) {
            ""
        } else {
            // Hable operates on float linear RGB. FFmpeg uses MaxCLL/mastering
            // peak when available, otherwise 1000 nits after linearization.
            "tonemap=tonemap=hable:desat=0,"
        };
        format!(
            "{BASE_FILTER},lavfi=[zscale=w=320:h=180:t=linear:npl=100:agamma=0,format=gbrpf32le,zscale=p=bt709,{tone}zscale=t=iec61966-2-1:p=bt709:r=full:dither=none,format=gbrp]:o=\"threads=2\""
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dolby_vision_requires_known_hdr10_base_layer() {
        assert!(matches!(
            Plan::classify("pq", "bt.2020-ncl", "bt.2020", "dolbyvision", Some(8)),
            Ok(Plan::DolbyVision8Base)
        ));
        assert!(Plan::classify("pq", "bt.2020-ncl", "bt.2020", "dolbyvision", Some(5)).is_err());
        assert!(Plan::classify("hlg", "bt.2020-ncl", "bt.2020", "dolbyvision", Some(8)).is_err());
        assert!(Plan::classify("pq", "bt.2020-ncl", "bt.2020", "dolbyvision", None).is_err());
        assert!(Plan::classify("v-log", "bt.2020-ncl", "bt.2020", "bt.2020-ncl", None).is_err());
    }
}
