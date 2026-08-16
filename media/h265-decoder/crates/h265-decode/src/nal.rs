//! HEVC NAL unit parsing: types, parameter sets (VPS, SPS, PPS), and slice
//! header parsing.
//!
//! This module implements the HEVC/H.265 NAL unit layer as specified in
//! ITU-T H.265 (04/2013) and later editions.  It covers:
//!
//! * **NAL unit header** (2 bytes): type, layer ID, temporal ID.
//! * **Video Parameter Set (VPS)** – ITU-T H.265 §7.3.2.1
//! * **Sequence Parameter Set (SPS)** – ITU-T H.265 §7.3.2.2
//! * **Picture Parameter Set (PPS)** – ITU-T H.265 §7.3.2.3
//! * **Slice segment header** (partial) – ITU-T H.265 §7.3.6.1
//!
//! The parser is intentionally tolerant: unknown or unsupported extensions
//! are skipped rather than rejected, and missing optional fields fall back
//! to spec defaults.

use crate::bitstream::BitstreamReader;
use crate::error::{DecodeError, DecodeResult};

// =========================================================================
// NAL unit header
// =========================================================================

/// HEVC NAL unit header (2 bytes).
///
/// ```text
/// +---------------+---------------+
/// |0|1 2 3 4 5 6|7 8 9 A B C|D E F|
/// |F| NalUnitType| nuh_layer_id |TID|
/// +---------------+---------------+
/// ```
///
/// * `forbidden_zero_bit` – u(1), must be 0.
/// * `nal_unit_type` – u(6), see [`NalUnitType`].
/// * `nuh_layer_id` – u(6), 0 for single-layer streams.
/// * `nuh_temporal_id_plus1` – u(3), temporal sub-layer + 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalHeader {
    /// The raw 6-bit NAL unit type value.
    pub nal_unit_type: u8,
    /// Layer identifier (0 for base layer).
    pub nuh_layer_id: u8,
    /// Temporal identifier plus one.  `temporal_id = nuh_temporal_id_plus1 - 1`.
    pub nuh_temporal_id_plus1: u8,
}

impl NalHeader {
    /// Parse a 2-byte NAL unit header from the start of `data`.
    ///
    /// `data` must contain at least 2 bytes (the NAL header bytes, *not*
    /// including any start-code prefix).
    pub fn parse(data: &[u8]) -> DecodeResult<Self> {
        if data.len() < 2 {
            return Err(DecodeError::InvalidBitstream(
                "NAL unit too short for 2-byte header".into(),
            ));
        }

        let forbidden = (data[0] >> 7) & 1;
        if forbidden != 0 {
            log::warn!("forbidden_zero_bit is set in NAL header");
        }

        let nal_unit_type = (data[0] >> 1) & 0x3F;
        let nuh_layer_id = ((data[0] & 1) << 5) | ((data[1] >> 3) & 0x1F);
        let nuh_temporal_id_plus1 = data[1] & 0x07;

        Ok(Self {
            nal_unit_type,
            nuh_layer_id,
            nuh_temporal_id_plus1,
        })
    }

    /// Temporal sub-layer index (0-based).
    #[inline]
    pub fn temporal_id(&self) -> u8 {
        self.nuh_temporal_id_plus1.saturating_sub(1)
    }

    /// Classify the NAL unit type.
    #[inline]
    pub fn nal_type(&self) -> NalUnitType {
        NalUnitType::from_raw(self.nal_unit_type)
    }
}

// =========================================================================
// NAL unit type enumeration
// =========================================================================

/// HEVC NAL unit types (ITU-T H.265 Table 7-1).
///
/// VCL types (0..31) carry coded slice data.  Non-VCL types (32..63)
/// carry parameter sets, SEI, and other metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NalUnitType {
    // -- VCL NAL unit types (coded slice segment) --
    /// Coded slice segment of a non-TSA, non-STSA trailing picture (N).
    TrailN,
    /// Coded slice segment of a non-TSA, non-STSA trailing picture (R).
    TrailR,
    /// Temporal sub-layer access, non-reference.
    TsaN,
    /// Temporal sub-layer access, reference.
    TsaR,
    /// Step-wise temporal sub-layer access, non-reference.
    StsaN,
    /// Step-wise temporal sub-layer access, reference.
    StsaR,
    /// Random access decodable leading, non-reference.
    RadlN,
    /// Random access decodable leading, reference.
    RadlR,
    /// Random access skipped leading, non-reference.
    RaslN,
    /// Random access skipped leading, reference.
    RaslR,
    /// Broken link access, W-type leading pictures.
    BlaWLp,
    /// Broken link access, W-type random access decodable leading.
    BlaWRadl,
    /// Broken link access, N-type leading pictures.
    BlaNLp,
    /// IDR with RADL pictures.
    IdrWRadl,
    /// IDR without leading pictures.
    IdrNLp,
    /// Clean random access.
    CraNut,

    // -- Non-VCL NAL unit types --
    /// Video Parameter Set.
    Vps,
    /// Sequence Parameter Set.
    Sps,
    /// Picture Parameter Set.
    Pps,
    /// Access Unit Delimiter.
    Aud,
    /// End of Sequence.
    EosNut,
    /// End of Bitstream.
    EobNut,
    /// Filler Data.
    FdNut,
    /// Supplemental Enhancement Information (prefix).
    SeiPrefix,
    /// Supplemental Enhancement Information (suffix).
    SeiSuffix,

    /// Reserved or unspecified NAL unit type.
    Other(u8),
}

impl NalUnitType {
    /// Convert a raw 6-bit NAL unit type value to the enum.
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 => NalUnitType::TrailN,
            1 => NalUnitType::TrailR,
            2 => NalUnitType::TsaN,
            3 => NalUnitType::TsaR,
            4 => NalUnitType::StsaN,
            5 => NalUnitType::StsaR,
            6 => NalUnitType::RadlN,
            7 => NalUnitType::RadlR,
            8 => NalUnitType::RaslN,
            9 => NalUnitType::RaslR,
            // 10..=15: reserved VCL
            16 => NalUnitType::BlaWLp,
            17 => NalUnitType::BlaWRadl,
            18 => NalUnitType::BlaNLp,
            19 => NalUnitType::IdrWRadl,
            20 => NalUnitType::IdrNLp,
            21 => NalUnitType::CraNut,
            // 22..=31: reserved IRAP / non-IRAP VCL
            32 => NalUnitType::Vps,
            33 => NalUnitType::Sps,
            34 => NalUnitType::Pps,
            35 => NalUnitType::Aud,
            36 => NalUnitType::EosNut,
            37 => NalUnitType::EobNut,
            38 => NalUnitType::FdNut,
            39 => NalUnitType::SeiPrefix,
            40 => NalUnitType::SeiSuffix,
            other => NalUnitType::Other(other),
        }
    }

    /// Raw 6-bit value.
    pub fn raw(self) -> u8 {
        match self {
            NalUnitType::TrailN => 0,
            NalUnitType::TrailR => 1,
            NalUnitType::TsaN => 2,
            NalUnitType::TsaR => 3,
            NalUnitType::StsaN => 4,
            NalUnitType::StsaR => 5,
            NalUnitType::RadlN => 6,
            NalUnitType::RadlR => 7,
            NalUnitType::RaslN => 8,
            NalUnitType::RaslR => 9,
            NalUnitType::BlaWLp => 16,
            NalUnitType::BlaWRadl => 17,
            NalUnitType::BlaNLp => 18,
            NalUnitType::IdrWRadl => 19,
            NalUnitType::IdrNLp => 20,
            NalUnitType::CraNut => 21,
            NalUnitType::Vps => 32,
            NalUnitType::Sps => 33,
            NalUnitType::Pps => 34,
            NalUnitType::Aud => 35,
            NalUnitType::EosNut => 36,
            NalUnitType::EobNut => 37,
            NalUnitType::FdNut => 38,
            NalUnitType::SeiPrefix => 39,
            NalUnitType::SeiSuffix => 40,
            NalUnitType::Other(v) => v,
        }
    }

    /// Returns `true` for VCL (Video Coding Layer) NAL unit types that
    /// carry coded slice data.
    pub fn is_vcl(self) -> bool {
        self.raw() <= 31
    }

    /// Returns `true` for slice NAL unit types (VCL types that produce
    /// decoded pictures).
    pub fn is_slice(self) -> bool {
        self.is_vcl()
    }

    /// Returns `true` for Intra Random Access Point (IRAP) pictures.
    ///
    /// IRAP types are: BLA_W_LP (16), BLA_W_RADL (17), BLA_N_LP (18),
    /// IDR_W_RADL (19), IDR_N_LP (20), CRA_NUT (21).
    pub fn is_irap(self) -> bool {
        let v = self.raw();
        (16..=21).contains(&v)
    }

    /// Returns `true` for IDR pictures (IDR_W_RADL or IDR_N_LP).
    pub fn is_idr(self) -> bool {
        matches!(self, NalUnitType::IdrWRadl | NalUnitType::IdrNLp)
    }

    /// Returns `true` for BLA (Broken Link Access) pictures.
    pub fn is_bla(self) -> bool {
        matches!(
            self,
            NalUnitType::BlaWLp | NalUnitType::BlaWRadl | NalUnitType::BlaNLp
        )
    }

    /// Returns `true` for CRA pictures.
    pub fn is_cra(self) -> bool {
        matches!(self, NalUnitType::CraNut)
    }

    /// Returns `true` for RADL (Random Access Decodable Leading) pictures.
    pub fn is_radl(self) -> bool {
        matches!(self, NalUnitType::RadlN | NalUnitType::RadlR)
    }

    /// Returns `true` for RASL (Random Access Skipped Leading) pictures.
    pub fn is_rasl(self) -> bool {
        matches!(self, NalUnitType::RaslN | NalUnitType::RaslR)
    }

    /// Returns `true` if this NAL type is a reference picture
    /// (the "R" variants have even-numbered types for TRAIL, TSA, etc.).
    ///
    /// More precisely, for types 0..9 the odd types are reference pictures.
    /// IRAP pictures are always treated as reference.
    pub fn is_reference(self) -> bool {
        if self.is_irap() {
            return true;
        }
        let v = self.raw();
        if v <= 9 {
            // Odd type values (1, 3, 5, 7, 9) are "R" (reference)
            v & 1 == 1
        } else {
            false
        }
    }
}

impl std::fmt::Display for NalUnitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NalUnitType::TrailN => write!(f, "TRAIL_N"),
            NalUnitType::TrailR => write!(f, "TRAIL_R"),
            NalUnitType::TsaN => write!(f, "TSA_N"),
            NalUnitType::TsaR => write!(f, "TSA_R"),
            NalUnitType::StsaN => write!(f, "STSA_N"),
            NalUnitType::StsaR => write!(f, "STSA_R"),
            NalUnitType::RadlN => write!(f, "RADL_N"),
            NalUnitType::RadlR => write!(f, "RADL_R"),
            NalUnitType::RaslN => write!(f, "RASL_N"),
            NalUnitType::RaslR => write!(f, "RASL_R"),
            NalUnitType::BlaWLp => write!(f, "BLA_W_LP"),
            NalUnitType::BlaWRadl => write!(f, "BLA_W_RADL"),
            NalUnitType::BlaNLp => write!(f, "BLA_N_LP"),
            NalUnitType::IdrWRadl => write!(f, "IDR_W_RADL"),
            NalUnitType::IdrNLp => write!(f, "IDR_N_LP"),
            NalUnitType::CraNut => write!(f, "CRA_NUT"),
            NalUnitType::Vps => write!(f, "VPS_NUT"),
            NalUnitType::Sps => write!(f, "SPS_NUT"),
            NalUnitType::Pps => write!(f, "PPS_NUT"),
            NalUnitType::Aud => write!(f, "AUD_NUT"),
            NalUnitType::EosNut => write!(f, "EOS_NUT"),
            NalUnitType::EobNut => write!(f, "EOB_NUT"),
            NalUnitType::FdNut => write!(f, "FD_NUT"),
            NalUnitType::SeiPrefix => write!(f, "PREFIX_SEI"),
            NalUnitType::SeiSuffix => write!(f, "SUFFIX_SEI"),
            NalUnitType::Other(v) => write!(f, "UNKNOWN({v})"),
        }
    }
}

// =========================================================================
// Profile / Tier / Level (PTL)  –  §7.3.3
// =========================================================================

/// General profile, tier, and level information (§7.3.3).
///
/// Carries the `general_profile_idc`, compatibility flags, constraint
/// indicators, tier flag, and level.
#[derive(Debug, Clone, Default)]
pub struct ProfileTierLevel {
    /// `general_profile_space` – u(2).
    pub general_profile_space: u8,
    /// `general_tier_flag` – u(1).
    pub general_tier_flag: bool,
    /// `general_profile_idc` – u(5).
    pub general_profile_idc: u8,
    /// `general_profile_compatibility_flag[j]` – 32 flags.
    pub general_profile_compatibility_flags: u32,
    /// `general_progressive_source_flag`.
    pub general_progressive_source_flag: bool,
    /// `general_interlaced_source_flag`.
    pub general_interlaced_source_flag: bool,
    /// `general_non_packed_constraint_flag`.
    pub general_non_packed_constraint_flag: bool,
    /// `general_frame_only_constraint_flag`.
    pub general_frame_only_constraint_flag: bool,
    /// 44 bits of reserved / constraint flags after the four above.
    pub general_constraint_indicator_flags: u64,
    /// `general_level_idc` – u(8).  Level × 30, e.g. 120 = Level 4.0.
    pub general_level_idc: u8,
}

impl ProfileTierLevel {
    /// Parse `profile_tier_level( 1, sps_max_sub_layers_minus1 )`.
    ///
    /// `profile_present_flag` is always `true` when called from VPS/SPS.
    /// `max_sub_layers_minus1` indicates how many sub-layer PTL entries
    /// follow.  We currently only parse the *general* PTL and skip
    /// sub-layer entries.
    pub fn parse(
        reader: &mut BitstreamReader<'_>,
        profile_present_flag: bool,
        max_sub_layers_minus1: u8,
    ) -> DecodeResult<Self> {
        let mut ptl = ProfileTierLevel::default();

        if profile_present_flag {
            ptl.general_profile_space = reader.read_bits(2)? as u8;
            ptl.general_tier_flag = reader.read_flag()?;
            ptl.general_profile_idc = reader.read_bits(5)? as u8;

            ptl.general_profile_compatibility_flags = reader.read_bits(32)?;

            ptl.general_progressive_source_flag = reader.read_flag()?;
            ptl.general_interlaced_source_flag = reader.read_flag()?;
            ptl.general_non_packed_constraint_flag = reader.read_flag()?;
            ptl.general_frame_only_constraint_flag = reader.read_flag()?;

            // 44 bits of constraint indicator flags / reserved zero bits.
            ptl.general_constraint_indicator_flags = reader.read_bits_u64(44)?;
        }

        ptl.general_level_idc = reader.read_bits(8)? as u8;

        // Sub-layer profile/level presence flags.
        let mut sub_layer_profile_present = [false; 8];
        let mut sub_layer_level_present = [false; 8];
        for i in 0..max_sub_layers_minus1 as usize {
            sub_layer_profile_present[i] = reader.read_flag()?;
            sub_layer_level_present[i] = reader.read_flag()?;
        }

        // Alignment padding when max_sub_layers_minus1 > 0.
        if max_sub_layers_minus1 > 0 {
            for _ in max_sub_layers_minus1..8 {
                reader.read_bits(2)?; // reserved_zero_2bits
            }
        }

        // Sub-layer PTL entries – skip them for now.
        for i in 0..max_sub_layers_minus1 as usize {
            if sub_layer_profile_present[i] {
                // sub_layer_profile_space, tier, idc, compat flags,
                // progressive/interlaced/non-packed/frame-only, 44 constraint bits
                reader.skip_bits(2 + 1 + 5 + 32 + 4 + 44)?;
            }
            if sub_layer_level_present[i] {
                reader.skip_bits(8)?; // sub_layer_level_idc
            }
        }

        Ok(ptl)
    }
}

// =========================================================================
// Short-term reference picture set  –  §7.3.7
// =========================================================================

/// A single short-term reference picture set (§7.3.7).
///
/// Each ST-RPS describes a set of reference pictures relative to the
/// current picture by their POC deltas.  These are signalled in the SPS
/// and optionally overridden in the slice header.
#[derive(Debug, Clone, Default)]
pub struct ShortTermRefPicSet {
    /// Number of negative-delta reference pictures.
    pub num_negative_pics: u32,
    /// Number of positive-delta reference pictures.
    pub num_positive_pics: u32,
    /// POC deltas for negative pictures (relative, cumulative).
    pub delta_poc_s0: Vec<i32>,
    /// Used-by-current flags for negative pictures.
    pub used_by_curr_pic_s0: Vec<bool>,
    /// POC deltas for positive pictures (relative, cumulative).
    pub delta_poc_s1: Vec<i32>,
    /// Used-by-current flags for positive pictures.
    pub used_by_curr_pic_s1: Vec<bool>,
}

impl ShortTermRefPicSet {
    /// Parse `st_ref_pic_set( stRpsIdx )` from the SPS.
    ///
    /// `st_rps_idx` is the index of this set in the SPS array.
    /// `rps_list` contains all previously parsed sets (for the
    /// `inter_ref_pic_set_prediction_flag` case).
    /// `num_short_term_ref_pic_sets` is the total count from the SPS.
    pub fn parse(
        reader: &mut BitstreamReader<'_>,
        st_rps_idx: usize,
        rps_list: &[ShortTermRefPicSet],
        _num_short_term_ref_pic_sets: u32,
    ) -> DecodeResult<Self> {
        let mut rps = ShortTermRefPicSet::default();

        let inter_ref_pic_set_prediction_flag = if st_rps_idx != 0 {
            reader.read_flag()?
        } else {
            false
        };

        if inter_ref_pic_set_prediction_flag {
            // Predicted from a previous RPS.
            let delta_idx_minus1 = if st_rps_idx == rps_list.len() {
                // Slice header case
                reader.read_ue()?
            } else {
                reader.read_ue()?
            };

            let delta_rps_sign = reader.read_flag()?;
            let abs_delta_rps_minus1 = reader.read_ue()?;

            let ref_rps_idx = (st_rps_idx as i32 - 1 - delta_idx_minus1 as i32) as usize;
            if ref_rps_idx >= rps_list.len() {
                return Err(DecodeError::InvalidBitstream(
                    "ST-RPS inter prediction references out-of-bounds set".into(),
                ));
            }
            let ref_rps = &rps_list[ref_rps_idx];

            let delta_rps = (1 - 2 * delta_rps_sign as i32) * (abs_delta_rps_minus1 as i32 + 1);

            let num_delta_pocs =
                ref_rps.num_negative_pics as usize + ref_rps.num_positive_pics as usize;

            let mut used_by_curr_pic_flag = Vec::with_capacity(num_delta_pocs + 1);
            let mut use_delta_flag = Vec::with_capacity(num_delta_pocs + 1);

            for _ in 0..=num_delta_pocs {
                let used = reader.read_flag()?;
                used_by_curr_pic_flag.push(used);
                if !used {
                    use_delta_flag.push(reader.read_flag()?);
                } else {
                    use_delta_flag.push(true);
                }
            }

            // Derive the new RPS from the reference RPS.
            // Collect all delta POC values from the reference set.
            let mut ref_delta_pocs: Vec<i32> = Vec::with_capacity(num_delta_pocs);
            for i in 0..ref_rps.num_negative_pics as usize {
                ref_delta_pocs.push(ref_rps.delta_poc_s0[i]);
            }
            for i in 0..ref_rps.num_positive_pics as usize {
                ref_delta_pocs.push(ref_rps.delta_poc_s1[i]);
            }

            // delta_poc_s0/s1 are cumulative in the reference; compute
            // the derived set.
            let mut neg_pocs: Vec<(i32, bool)> = Vec::new();
            let mut pos_pocs: Vec<(i32, bool)> = Vec::new();

            // Entry for delta_rps itself (j = num_delta_pocs)
            if use_delta_flag[num_delta_pocs] {
                let d_poc = delta_rps;
                let used = used_by_curr_pic_flag[num_delta_pocs];
                if d_poc < 0 {
                    neg_pocs.push((d_poc, used));
                } else if d_poc > 0 {
                    pos_pocs.push((d_poc, used));
                }
            }

            for j in 0..num_delta_pocs {
                if use_delta_flag[j] {
                    let d_poc = ref_delta_pocs[j] + delta_rps;
                    let used = used_by_curr_pic_flag[j];
                    if d_poc < 0 {
                        neg_pocs.push((d_poc, used));
                    } else if d_poc > 0 {
                        pos_pocs.push((d_poc, used));
                    }
                }
            }

            // Sort: negative POCs descending (closest to 0 first),
            // positive ascending.
            neg_pocs.sort_by(|a, b| b.0.cmp(&a.0));
            pos_pocs.sort_by(|a, b| a.0.cmp(&b.0));

            rps.num_negative_pics = neg_pocs.len() as u32;
            rps.num_positive_pics = pos_pocs.len() as u32;
            rps.delta_poc_s0 = neg_pocs.iter().map(|&(d, _)| d).collect();
            rps.used_by_curr_pic_s0 = neg_pocs.iter().map(|&(_, u)| u).collect();
            rps.delta_poc_s1 = pos_pocs.iter().map(|&(d, _)| d).collect();
            rps.used_by_curr_pic_s1 = pos_pocs.iter().map(|&(_, u)| u).collect();
        } else {
            // Explicit signalling.
            rps.num_negative_pics = reader.read_ue()?;
            rps.num_positive_pics = reader.read_ue()?;

            // Sanity check
            if rps.num_negative_pics > 16 || rps.num_positive_pics > 16 {
                return Err(DecodeError::InvalidBitstream(format!(
                    "ST-RPS num_pics too large: neg={}, pos={}",
                    rps.num_negative_pics, rps.num_positive_pics,
                )));
            }

            let mut prev = 0i32;
            for _ in 0..rps.num_negative_pics {
                let delta_poc_s0_minus1 = reader.read_ue()?;
                let used = reader.read_flag()?;
                prev -= (delta_poc_s0_minus1 as i32) + 1;
                rps.delta_poc_s0.push(prev);
                rps.used_by_curr_pic_s0.push(used);
            }

            prev = 0;
            for _ in 0..rps.num_positive_pics {
                let delta_poc_s1_minus1 = reader.read_ue()?;
                let used = reader.read_flag()?;
                prev += (delta_poc_s1_minus1 as i32) + 1;
                rps.delta_poc_s1.push(prev);
                rps.used_by_curr_pic_s1.push(used);
            }
        }

        Ok(rps)
    }

    /// Total number of pictures in this set.
    pub fn num_delta_pocs(&self) -> u32 {
        self.num_negative_pics + self.num_positive_pics
    }
}

// =========================================================================
// Video Parameter Set (VPS)  –  §7.3.2.1
// =========================================================================

/// Decoded Video Parameter Set.
///
/// The VPS carries high-level information about the coded video sequence:
/// the number of temporal sub-layers, layer configuration, and basic
/// timing information.
#[derive(Debug, Clone)]
pub struct Vps {
    /// `vps_video_parameter_set_id` – u(4).
    pub vps_id: u8,
    /// `vps_base_layer_internal_flag` – u(1).
    pub base_layer_internal_flag: bool,
    /// `vps_base_layer_available_flag` – u(1).
    pub base_layer_available_flag: bool,
    /// `vps_max_layers_minus1` – u(6).
    pub max_layers_minus1: u8,
    /// `vps_max_sub_layers_minus1` – u(3).
    pub max_sub_layers_minus1: u8,
    /// `vps_temporal_id_nesting_flag` – u(1).
    pub temporal_id_nesting_flag: bool,
    /// General profile / tier / level.
    pub profile_tier_level: ProfileTierLevel,
    /// `vps_max_dec_pic_buffering_minus1[i]` for each sub-layer.
    pub max_dec_pic_buffering_minus1: Vec<u32>,
    /// `vps_max_num_reorder_pics[i]` for each sub-layer.
    pub max_num_reorder_pics: Vec<u32>,
    /// `vps_max_latency_increase_plus1[i]` for each sub-layer.
    pub max_latency_increase_plus1: Vec<u32>,
}

/// Key-indexed store for VPS instances.
pub struct VpsStore {
    sets: Vec<Option<Vps>>,
}

impl VpsStore {
    pub fn new() -> Self {
        Self {
            sets: vec![None; 16],
        }
    }

    pub fn insert(&mut self, id: u8, vps: Vps) {
        if (id as usize) < self.sets.len() {
            self.sets[id as usize] = Some(vps);
        }
    }

    pub fn get(&self, id: u8) -> Option<&Vps> {
        self.sets.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn clear(&mut self) {
        for s in &mut self.sets {
            *s = None;
        }
    }
}

impl Default for VpsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a VPS NAL unit from RBSP data (after the 2-byte NAL header and
/// emulation-prevention removal).
pub fn parse_vps(rbsp: &[u8]) -> DecodeResult<Vps> {
    let mut r = BitstreamReader::new(rbsp);

    let vps_id = r.read_bits(4)? as u8;
    let base_layer_internal_flag = r.read_flag()?;
    let base_layer_available_flag = r.read_flag()?;
    let max_layers_minus1 = r.read_bits(6)? as u8;
    let max_sub_layers_minus1 = r.read_bits(3)? as u8;
    let temporal_id_nesting_flag = r.read_flag()?;

    // reserved_0xffff_16bits
    r.skip_bits(16)?;

    let ptl = ProfileTierLevel::parse(&mut r, true, max_sub_layers_minus1)?;

    let sub_layer_ordering_info_present_flag = r.read_flag()?;
    let start = if sub_layer_ordering_info_present_flag {
        0
    } else {
        max_sub_layers_minus1 as usize
    };

    let num_layers = (max_sub_layers_minus1 as usize) + 1;
    let mut max_dec_pic_buffering_minus1 = vec![0u32; num_layers];
    let mut max_num_reorder_pics = vec![0u32; num_layers];
    let mut max_latency_increase_plus1 = vec![0u32; num_layers];

    for i in start..num_layers {
        max_dec_pic_buffering_minus1[i] = r.read_ue()?;
        max_num_reorder_pics[i] = r.read_ue()?;
        max_latency_increase_plus1[i] = r.read_ue()?;
    }

    // Fill lower sub-layers if ordering info was only present for the
    // highest sub-layer.
    if !sub_layer_ordering_info_present_flag {
        let v_dpb = max_dec_pic_buffering_minus1[start];
        let v_reorder = max_num_reorder_pics[start];
        let v_latency = max_latency_increase_plus1[start];
        for i in 0..start {
            max_dec_pic_buffering_minus1[i] = v_dpb;
            max_num_reorder_pics[i] = v_reorder;
            max_latency_increase_plus1[i] = v_latency;
        }
    }

    // We skip the rest of the VPS (layer sets, timing info, extensions)
    // as they are not needed for basic decoding.

    Ok(Vps {
        vps_id,
        base_layer_internal_flag,
        base_layer_available_flag,
        max_layers_minus1,
        max_sub_layers_minus1,
        temporal_id_nesting_flag,
        profile_tier_level: ptl,
        max_dec_pic_buffering_minus1,
        max_num_reorder_pics,
        max_latency_increase_plus1,
    })
}

// =========================================================================
// Sequence Parameter Set (SPS)  –  §7.3.2.2
// =========================================================================

/// Scaling list data placeholder.
///
/// Full scaling list parsing is deferred; we store the raw presence flag
/// and skip the data in the bitstream.
#[derive(Debug, Clone, Default)]
pub struct ScalingListData {
    /// Whether scaling list data was present in the bitstream.
    pub present: bool,
}

/// Decoded Sequence Parameter Set.
///
/// Contains all picture-level parameters: resolution, chroma format,
/// bit depth, coding structure limits, reference picture sets, and more.
#[derive(Debug, Clone)]
pub struct Sps {
    /// `sps_video_parameter_set_id` – u(4).
    pub vps_id: u8,
    /// `sps_max_sub_layers_minus1` – u(3).
    pub max_sub_layers_minus1: u8,
    /// `sps_temporal_id_nesting_flag` – u(1).
    pub temporal_id_nesting_flag: bool,
    /// General profile / tier / level.
    pub profile_tier_level: ProfileTierLevel,
    /// `sps_seq_parameter_set_id` – ue(v).
    pub sps_id: u32,
    /// `chroma_format_idc` – ue(v).  0=mono, 1=4:2:0, 2=4:2:2, 3=4:4:4.
    pub chroma_format_idc: u8,
    /// `separate_colour_plane_flag`.
    pub separate_colour_plane_flag: bool,
    /// `pic_width_in_luma_samples` – ue(v).
    pub pic_width_in_luma_samples: u32,
    /// `pic_height_in_luma_samples` – ue(v).
    pub pic_height_in_luma_samples: u32,
    /// Conformance window offsets (in luma samples, already multiplied by
    /// the sub-width/sub-height factors).
    pub conf_win_left_offset: u32,
    pub conf_win_right_offset: u32,
    pub conf_win_top_offset: u32,
    pub conf_win_bottom_offset: u32,
    /// `bit_depth_luma_minus8` + 8.
    pub bit_depth_luma: u8,
    /// `bit_depth_chroma_minus8` + 8.
    pub bit_depth_chroma: u8,
    /// `log2_max_pic_order_cnt_lsb_minus4` + 4.
    pub log2_max_pic_order_cnt_lsb: u8,
    /// Sub-layer ordering info.
    pub max_dec_pic_buffering_minus1: Vec<u32>,
    pub max_num_reorder_pics: Vec<u32>,
    pub max_latency_increase_plus1: Vec<u32>,
    /// `log2_min_luma_coding_block_size_minus3` + 3.
    pub log2_min_luma_coding_block_size: u8,
    /// `log2_diff_max_min_luma_coding_block_size`.
    pub log2_diff_max_min_luma_coding_block_size: u8,
    /// `log2_min_luma_transform_block_size_minus2` + 2.
    pub log2_min_luma_transform_block_size: u8,
    /// `log2_diff_max_min_luma_transform_block_size`.
    pub log2_diff_max_min_luma_transform_block_size: u8,
    /// `max_transform_hierarchy_depth_inter`.
    pub max_transform_hierarchy_depth_inter: u32,
    /// `max_transform_hierarchy_depth_intra`.
    pub max_transform_hierarchy_depth_intra: u32,
    /// `scaling_list_enabled_flag`.
    pub scaling_list_enabled_flag: bool,
    /// Scaling list data (if present).
    pub scaling_list_data: ScalingListData,
    /// `amp_enabled_flag`.
    pub amp_enabled_flag: bool,
    /// `sample_adaptive_offset_enabled_flag`.
    pub sample_adaptive_offset_enabled_flag: bool,
    /// `pcm_enabled_flag`.
    pub pcm_enabled_flag: bool,
    /// PCM sample bit depths (only valid if pcm_enabled_flag).
    pub pcm_sample_bit_depth_luma_minus1: u8,
    pub pcm_sample_bit_depth_chroma_minus1: u8,
    pub log2_min_pcm_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_pcm_luma_coding_block_size: u8,
    pub pcm_loop_filter_disabled_flag: bool,
    /// Short-term reference picture sets defined in the SPS.
    pub short_term_ref_pic_sets: Vec<ShortTermRefPicSet>,
    /// `long_term_ref_pics_present_flag`.
    pub long_term_ref_pics_present_flag: bool,
    /// Number of long-term reference pictures signalled in the SPS.
    pub num_long_term_ref_pics_sps: u32,
    /// POC LSB values for SPS-signalled long-term reference pictures.
    pub lt_ref_pic_poc_lsb_sps: Vec<u32>,
    /// Used-by-current flags for long-term reference pictures.
    pub used_by_curr_pic_lt_sps_flag: Vec<bool>,
    /// `sps_temporal_mvp_enabled_flag`.
    pub temporal_mvp_enabled_flag: bool,
    /// `strong_intra_smoothing_enabled_flag`.
    pub strong_intra_smoothing_enabled_flag: bool,
    // VUI parameters are skipped in this implementation.
}

/// Key-indexed store for SPS instances.
pub struct SpsStore {
    sets: Vec<Option<Sps>>,
}

impl SpsStore {
    pub fn new() -> Self {
        Self {
            sets: vec![None; 16],
        }
    }

    pub fn insert(&mut self, id: u32, sps: Sps) {
        if (id as usize) < self.sets.len() {
            self.sets[id as usize] = Some(sps);
        }
    }

    pub fn get(&self, id: u32) -> Option<&Sps> {
        self.sets.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn clear(&mut self) {
        for s in &mut self.sets {
            *s = None;
        }
    }
}

impl Default for SpsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse an SPS NAL unit from RBSP data.
pub fn parse_sps(rbsp: &[u8]) -> DecodeResult<Sps> {
    let mut r = BitstreamReader::new(rbsp);

    let vps_id = r.read_bits(4)? as u8;
    let max_sub_layers_minus1 = r.read_bits(3)? as u8;
    let temporal_id_nesting_flag = r.read_flag()?;

    let ptl = ProfileTierLevel::parse(&mut r, true, max_sub_layers_minus1)?;

    let sps_id = r.read_ue()?;
    let chroma_format_idc = r.read_ue()? as u8;

    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        r.read_flag()?
    } else {
        false
    };

    let pic_width_in_luma_samples = r.read_ue()?;
    let pic_height_in_luma_samples = r.read_ue()?;

    // Conformance window
    let conformance_window_flag = r.read_flag()?;
    let (mut conf_win_left, mut conf_win_right, mut conf_win_top, mut conf_win_bottom) =
        (0u32, 0u32, 0u32, 0u32);
    if conformance_window_flag {
        conf_win_left = r.read_ue()?;
        conf_win_right = r.read_ue()?;
        conf_win_top = r.read_ue()?;
        conf_win_bottom = r.read_ue()?;

        // Convert from coded-unit offsets to luma sample offsets.
        let sub_width_c: u32 = match chroma_format_idc {
            1 | 2 => 2,
            _ => 1,
        };
        let sub_height_c: u32 = match chroma_format_idc {
            1 => 2,
            _ => 1,
        };
        conf_win_left *= sub_width_c;
        conf_win_right *= sub_width_c;
        conf_win_top *= sub_height_c;
        conf_win_bottom *= sub_height_c;
    }

    let bit_depth_luma = r.read_ue()? as u8 + 8;
    let bit_depth_chroma = r.read_ue()? as u8 + 8;
    let log2_max_pic_order_cnt_lsb = r.read_ue()? as u8 + 4;

    // Sub-layer ordering info.
    let sub_layer_ordering_info_present_flag = r.read_flag()?;
    let num_layers = (max_sub_layers_minus1 as usize) + 1;
    let start = if sub_layer_ordering_info_present_flag {
        0
    } else {
        max_sub_layers_minus1 as usize
    };

    let mut max_dec_pic_buffering_minus1 = vec![0u32; num_layers];
    let mut max_num_reorder_pics = vec![0u32; num_layers];
    let mut max_latency_increase_plus1 = vec![0u32; num_layers];

    for i in start..num_layers {
        max_dec_pic_buffering_minus1[i] = r.read_ue()?;
        max_num_reorder_pics[i] = r.read_ue()?;
        max_latency_increase_plus1[i] = r.read_ue()?;
    }

    if !sub_layer_ordering_info_present_flag {
        let v0 = max_dec_pic_buffering_minus1[start];
        let v1 = max_num_reorder_pics[start];
        let v2 = max_latency_increase_plus1[start];
        for i in 0..start {
            max_dec_pic_buffering_minus1[i] = v0;
            max_num_reorder_pics[i] = v1;
            max_latency_increase_plus1[i] = v2;
        }
    }

    let log2_min_luma_coding_block_size = r.read_ue()? as u8 + 3;
    let log2_diff_max_min_luma_coding_block_size = r.read_ue()? as u8;
    let log2_min_luma_transform_block_size = r.read_ue()? as u8 + 2;
    let log2_diff_max_min_luma_transform_block_size = r.read_ue()? as u8;
    let max_transform_hierarchy_depth_inter = r.read_ue()?;
    let max_transform_hierarchy_depth_intra = r.read_ue()?;

    // Scaling lists
    let scaling_list_enabled_flag = r.read_flag()?;
    let mut scaling_list_data = ScalingListData::default();
    if scaling_list_enabled_flag {
        let sps_scaling_list_data_present_flag = r.read_flag()?;
        if sps_scaling_list_data_present_flag {
            scaling_list_data.present = true;
            skip_scaling_list_data(&mut r)?;
        }
    }

    let amp_enabled_flag = r.read_flag()?;
    let sample_adaptive_offset_enabled_flag = r.read_flag()?;

    let pcm_enabled_flag = r.read_flag()?;
    let mut pcm_sample_bit_depth_luma_minus1 = 0u8;
    let mut pcm_sample_bit_depth_chroma_minus1 = 0u8;
    let mut log2_min_pcm_luma_coding_block_size_minus3 = 0u8;
    let mut log2_diff_max_min_pcm_luma_coding_block_size = 0u8;
    let mut pcm_loop_filter_disabled_flag = false;

    if pcm_enabled_flag {
        pcm_sample_bit_depth_luma_minus1 = r.read_bits(4)? as u8;
        pcm_sample_bit_depth_chroma_minus1 = r.read_bits(4)? as u8;
        log2_min_pcm_luma_coding_block_size_minus3 = r.read_ue()? as u8;
        log2_diff_max_min_pcm_luma_coding_block_size = r.read_ue()? as u8;
        pcm_loop_filter_disabled_flag = r.read_flag()?;
    }

    // Short-term reference picture sets.
    let num_short_term_ref_pic_sets = r.read_ue()?;
    if num_short_term_ref_pic_sets > 64 {
        return Err(DecodeError::InvalidBitstream(format!(
            "num_short_term_ref_pic_sets={num_short_term_ref_pic_sets} exceeds 64"
        )));
    }

    let mut short_term_ref_pic_sets = Vec::with_capacity(num_short_term_ref_pic_sets as usize);
    for i in 0..num_short_term_ref_pic_sets as usize {
        let rps = ShortTermRefPicSet::parse(
            &mut r,
            i,
            &short_term_ref_pic_sets,
            num_short_term_ref_pic_sets,
        )?;
        short_term_ref_pic_sets.push(rps);
    }

    // Long-term reference pictures.
    let long_term_ref_pics_present_flag = r.read_flag()?;
    let mut num_long_term_ref_pics_sps = 0u32;
    let mut lt_ref_pic_poc_lsb_sps = Vec::new();
    let mut used_by_curr_pic_lt_sps_flag = Vec::new();

    if long_term_ref_pics_present_flag {
        num_long_term_ref_pics_sps = r.read_ue()?;
        for _ in 0..num_long_term_ref_pics_sps {
            lt_ref_pic_poc_lsb_sps.push(r.read_bits(log2_max_pic_order_cnt_lsb)?);
            used_by_curr_pic_lt_sps_flag.push(r.read_flag()?);
        }
    }

    let temporal_mvp_enabled_flag = r.read_flag()?;
    let strong_intra_smoothing_enabled_flag = r.read_flag()?;

    // We skip VUI parameters and SPS extensions for now.

    Ok(Sps {
        vps_id,
        max_sub_layers_minus1,
        temporal_id_nesting_flag,
        profile_tier_level: ptl,
        sps_id,
        chroma_format_idc,
        separate_colour_plane_flag,
        pic_width_in_luma_samples,
        pic_height_in_luma_samples,
        conf_win_left_offset: conf_win_left,
        conf_win_right_offset: conf_win_right,
        conf_win_top_offset: conf_win_top,
        conf_win_bottom_offset: conf_win_bottom,
        bit_depth_luma,
        bit_depth_chroma,
        log2_max_pic_order_cnt_lsb,
        max_dec_pic_buffering_minus1,
        max_num_reorder_pics,
        max_latency_increase_plus1,
        log2_min_luma_coding_block_size,
        log2_diff_max_min_luma_coding_block_size,
        log2_min_luma_transform_block_size,
        log2_diff_max_min_luma_transform_block_size,
        max_transform_hierarchy_depth_inter,
        max_transform_hierarchy_depth_intra,
        scaling_list_enabled_flag,
        scaling_list_data,
        amp_enabled_flag,
        sample_adaptive_offset_enabled_flag,
        pcm_enabled_flag,
        pcm_sample_bit_depth_luma_minus1,
        pcm_sample_bit_depth_chroma_minus1,
        log2_min_pcm_luma_coding_block_size_minus3,
        log2_diff_max_min_pcm_luma_coding_block_size,
        pcm_loop_filter_disabled_flag,
        short_term_ref_pic_sets,
        long_term_ref_pics_present_flag,
        num_long_term_ref_pics_sps,
        lt_ref_pic_poc_lsb_sps,
        used_by_curr_pic_lt_sps_flag,
        temporal_mvp_enabled_flag,
        strong_intra_smoothing_enabled_flag,
    })
}

/// Helper: compute the cropped output width from an SPS.
pub fn sps_cropped_width(sps: &Sps) -> u32 {
    sps.pic_width_in_luma_samples - sps.conf_win_left_offset - sps.conf_win_right_offset
}

/// Helper: compute the cropped output height from an SPS.
pub fn sps_cropped_height(sps: &Sps) -> u32 {
    sps.pic_height_in_luma_samples - sps.conf_win_top_offset - sps.conf_win_bottom_offset
}

/// Helper: max CTB size log2.
pub fn sps_ctb_size_log2(sps: &Sps) -> u8 {
    sps.log2_min_luma_coding_block_size + sps.log2_diff_max_min_luma_coding_block_size
}

/// Helper: CTB size in luma samples.
pub fn sps_ctb_size(sps: &Sps) -> u32 {
    1 << sps_ctb_size_log2(sps)
}

/// Helper: picture width in CTBs.
pub fn sps_pic_width_in_ctbs(sps: &Sps) -> u32 {
    sps.pic_width_in_luma_samples.div_ceil(sps_ctb_size(sps))
}

/// Helper: picture height in CTBs.
pub fn sps_pic_height_in_ctbs(sps: &Sps) -> u32 {
    sps.pic_height_in_luma_samples.div_ceil(sps_ctb_size(sps))
}

/// Helper: total number of CTBs in the picture.
pub fn sps_pic_size_in_ctbs(sps: &Sps) -> u32 {
    sps_pic_width_in_ctbs(sps) * sps_pic_height_in_ctbs(sps)
}

/// Skip scaling_list_data() in the bitstream.
fn skip_scaling_list_data(r: &mut BitstreamReader<'_>) -> DecodeResult<()> {
    for size_id in 0..4u8 {
        let count = if size_id == 3 { 2 } else { 6 };
        for _ in 0..count {
            let pred_mode_flag = r.read_flag()?;
            if !pred_mode_flag {
                r.read_ue()?; // scaling_list_pred_matrix_id_delta
            } else {
                let coef_num: u32 = std::cmp::min(64, 1u32 << (4 + (size_id << 1)));
                if size_id > 1 {
                    r.read_se()?; // scaling_list_dc_coef_minus8
                }
                for _ in 0..coef_num {
                    r.read_se()?; // scaling_list_delta_coef
                }
            }
        }
    }
    Ok(())
}

// =========================================================================
// Picture Parameter Set (PPS)  –  §7.3.2.3
// =========================================================================

/// Decoded Picture Parameter Set.
///
/// The PPS carries slice-group-level parameters: entropy coding mode,
/// default reference index counts, QP settings, deblocking control, etc.
#[derive(Debug, Clone)]
pub struct Pps {
    /// `pps_pic_parameter_set_id` – ue(v).
    pub pps_id: u32,
    /// `pps_seq_parameter_set_id` – ue(v).
    pub sps_id: u32,
    /// `dependent_slice_segments_enabled_flag`.
    pub dependent_slice_segments_enabled_flag: bool,
    /// `output_flag_present_flag`.
    pub output_flag_present_flag: bool,
    /// `num_extra_slice_header_bits` – u(3).
    pub num_extra_slice_header_bits: u8,
    /// `sign_data_hiding_enabled_flag`.
    pub sign_data_hiding_enabled_flag: bool,
    /// `cabac_init_present_flag`.
    pub cabac_init_present_flag: bool,
    /// `num_ref_idx_l0_default_active_minus1`.
    pub num_ref_idx_l0_default_active_minus1: u32,
    /// `num_ref_idx_l1_default_active_minus1`.
    pub num_ref_idx_l1_default_active_minus1: u32,
    /// `init_qp_minus26` – se(v).
    pub init_qp_minus26: i32,
    /// `constrained_intra_pred_flag`.
    pub constrained_intra_pred_flag: bool,
    /// `transform_skip_enabled_flag`.
    pub transform_skip_enabled_flag: bool,
    /// `cu_qp_delta_enabled_flag`.
    pub cu_qp_delta_enabled_flag: bool,
    /// `diff_cu_qp_delta_depth`.
    pub diff_cu_qp_delta_depth: u32,
    /// `pps_cb_qp_offset` – se(v).
    pub cb_qp_offset: i32,
    /// `pps_cr_qp_offset` – se(v).
    pub cr_qp_offset: i32,
    /// `pps_slice_chroma_qp_offsets_present_flag`.
    pub slice_chroma_qp_offsets_present_flag: bool,
    /// `weighted_pred_flag`.
    pub weighted_pred_flag: bool,
    /// `weighted_bipred_flag`.
    pub weighted_bipred_flag: bool,
    /// `transquant_bypass_enabled_flag`.
    pub transquant_bypass_enabled_flag: bool,
    /// `tiles_enabled_flag`.
    pub tiles_enabled_flag: bool,
    /// `entropy_coding_sync_enabled_flag`.
    pub entropy_coding_sync_enabled_flag: bool,
    /// Number of tile columns.
    pub num_tile_columns_minus1: u32,
    /// Number of tile rows.
    pub num_tile_rows_minus1: u32,
    /// `uniform_spacing_flag`.
    pub uniform_spacing_flag: bool,
    /// Column widths (in CTBs) when non-uniform.
    pub column_width_minus1: Vec<u32>,
    /// Row heights (in CTBs) when non-uniform.
    pub row_height_minus1: Vec<u32>,
    /// `loop_filter_across_tiles_enabled_flag`.
    pub loop_filter_across_tiles_enabled_flag: bool,
    /// `pps_loop_filter_across_slices_enabled_flag`.
    pub loop_filter_across_slices_enabled_flag: bool,
    /// `deblocking_filter_control_present_flag`.
    pub deblocking_filter_control_present_flag: bool,
    /// `deblocking_filter_override_enabled_flag`.
    pub deblocking_filter_override_enabled_flag: bool,
    /// `pps_deblocking_filter_disabled_flag`.
    pub deblocking_filter_disabled_flag: bool,
    /// `pps_beta_offset_div2` – se(v).
    pub beta_offset_div2: i32,
    /// `pps_tc_offset_div2` – se(v).
    pub tc_offset_div2: i32,
    /// `lists_modification_present_flag`.
    pub lists_modification_present_flag: bool,
    /// `log2_parallel_merge_level_minus2`.
    pub log2_parallel_merge_level_minus2: u32,
    /// `slice_segment_header_extension_present_flag`.
    pub slice_segment_header_extension_present_flag: bool,
}

/// Key-indexed store for PPS instances.
pub struct PpsStore {
    sets: Vec<Option<Pps>>,
}

impl PpsStore {
    pub fn new() -> Self {
        Self {
            sets: vec![None; 64],
        }
    }

    pub fn insert(&mut self, id: u32, pps: Pps) {
        if (id as usize) < self.sets.len() {
            self.sets[id as usize] = Some(pps);
        }
    }

    pub fn get(&self, id: u32) -> Option<&Pps> {
        self.sets.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn clear(&mut self) {
        for s in &mut self.sets {
            *s = None;
        }
    }
}

impl Default for PpsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a PPS NAL unit from RBSP data.
pub fn parse_pps(rbsp: &[u8]) -> DecodeResult<Pps> {
    let mut r = BitstreamReader::new(rbsp);

    let pps_id = r.read_ue()?;
    let sps_id = r.read_ue()?;
    let dependent_slice_segments_enabled_flag = r.read_flag()?;
    let output_flag_present_flag = r.read_flag()?;
    let num_extra_slice_header_bits = r.read_bits(3)? as u8;
    let sign_data_hiding_enabled_flag = r.read_flag()?;
    let cabac_init_present_flag = r.read_flag()?;
    let num_ref_idx_l0_default_active_minus1 = r.read_ue()?;
    let num_ref_idx_l1_default_active_minus1 = r.read_ue()?;
    let init_qp_minus26 = r.read_se()?;
    let constrained_intra_pred_flag = r.read_flag()?;
    let transform_skip_enabled_flag = r.read_flag()?;

    let cu_qp_delta_enabled_flag = r.read_flag()?;
    let diff_cu_qp_delta_depth = if cu_qp_delta_enabled_flag {
        r.read_ue()?
    } else {
        0
    };

    let cb_qp_offset = r.read_se()?;
    let cr_qp_offset = r.read_se()?;
    let slice_chroma_qp_offsets_present_flag = r.read_flag()?;
    let weighted_pred_flag = r.read_flag()?;
    let weighted_bipred_flag = r.read_flag()?;
    let transquant_bypass_enabled_flag = r.read_flag()?;
    let tiles_enabled_flag = r.read_flag()?;
    let entropy_coding_sync_enabled_flag = r.read_flag()?;

    let mut num_tile_columns_minus1 = 0u32;
    let mut num_tile_rows_minus1 = 0u32;
    let mut uniform_spacing_flag = true;
    let mut column_width_minus1 = Vec::new();
    let mut row_height_minus1 = Vec::new();
    let mut loop_filter_across_tiles_enabled_flag = true;

    if tiles_enabled_flag {
        num_tile_columns_minus1 = r.read_ue()?;
        num_tile_rows_minus1 = r.read_ue()?;
        uniform_spacing_flag = r.read_flag()?;
        if !uniform_spacing_flag {
            for _ in 0..num_tile_columns_minus1 {
                column_width_minus1.push(r.read_ue()?);
            }
            for _ in 0..num_tile_rows_minus1 {
                row_height_minus1.push(r.read_ue()?);
            }
        }
        loop_filter_across_tiles_enabled_flag = r.read_flag()?;
    }

    let loop_filter_across_slices_enabled_flag = r.read_flag()?;

    let deblocking_filter_control_present_flag = r.read_flag()?;
    let mut deblocking_filter_override_enabled_flag = false;
    let mut deblocking_filter_disabled_flag = false;
    let mut beta_offset_div2 = 0i32;
    let mut tc_offset_div2 = 0i32;

    if deblocking_filter_control_present_flag {
        deblocking_filter_override_enabled_flag = r.read_flag()?;
        deblocking_filter_disabled_flag = r.read_flag()?;
        if !deblocking_filter_disabled_flag {
            beta_offset_div2 = r.read_se()?;
            tc_offset_div2 = r.read_se()?;
        }
    }

    // Scaling list data override for PPS
    let pps_scaling_list_data_present_flag = r.read_flag()?;
    if pps_scaling_list_data_present_flag {
        skip_scaling_list_data(&mut r)?;
    }

    let lists_modification_present_flag = r.read_flag()?;
    let log2_parallel_merge_level_minus2 = r.read_ue()?;
    let slice_segment_header_extension_present_flag = r.read_flag()?;

    // Skip PPS extension flags and data.

    Ok(Pps {
        pps_id,
        sps_id,
        dependent_slice_segments_enabled_flag,
        output_flag_present_flag,
        num_extra_slice_header_bits,
        sign_data_hiding_enabled_flag,
        cabac_init_present_flag,
        num_ref_idx_l0_default_active_minus1,
        num_ref_idx_l1_default_active_minus1,
        init_qp_minus26,
        constrained_intra_pred_flag,
        transform_skip_enabled_flag,
        cu_qp_delta_enabled_flag,
        diff_cu_qp_delta_depth,
        cb_qp_offset,
        cr_qp_offset,
        slice_chroma_qp_offsets_present_flag,
        weighted_pred_flag,
        weighted_bipred_flag,
        transquant_bypass_enabled_flag,
        tiles_enabled_flag,
        entropy_coding_sync_enabled_flag,
        num_tile_columns_minus1,
        num_tile_rows_minus1,
        uniform_spacing_flag,
        column_width_minus1,
        row_height_minus1,
        loop_filter_across_tiles_enabled_flag,
        loop_filter_across_slices_enabled_flag,
        deblocking_filter_control_present_flag,
        deblocking_filter_override_enabled_flag,
        deblocking_filter_disabled_flag,
        beta_offset_div2,
        tc_offset_div2,
        lists_modification_present_flag,
        log2_parallel_merge_level_minus2,
        slice_segment_header_extension_present_flag,
    })
}

// =========================================================================
// Slice segment header (partial)  –  §7.3.6.1
// =========================================================================

/// Partially parsed slice segment header.
///
/// We extract only the fields needed for picture management and display
/// ordering: PPS id, slice type, POC LSB, and reference picture set
/// information.
#[derive(Debug, Clone)]
pub struct SliceHeader {
    /// `first_slice_segment_in_pic_flag`.
    pub first_slice_segment_in_pic_flag: bool,
    /// For IRAP pictures: `no_output_of_prior_pics_flag`.
    pub no_output_of_prior_pics_flag: bool,
    /// `slice_pic_parameter_set_id`.
    pub pps_id: u32,
    /// `dependent_slice_segment_flag`.
    pub dependent_slice_segment_flag: bool,
    /// `slice_segment_address` (only if not first slice).
    pub slice_segment_address: u32,
    /// `slice_type`: 0=B, 1=P, 2=I.
    pub slice_type: u8,
    /// `pic_output_flag` (if output_flag_present_flag in PPS).
    pub pic_output_flag: bool,
    /// `colour_plane_id` (only if separate_colour_plane_flag).
    pub colour_plane_id: u8,
    /// `slice_pic_order_cnt_lsb`.
    pub pic_order_cnt_lsb: u32,
    /// Index of the short-term RPS used (from SPS or slice-local).
    pub short_term_ref_pic_set_idx: u32,
    /// Whether a slice-local short-term RPS was signalled.
    pub short_term_ref_pic_set_sps_flag: bool,
    /// The short-term RPS (either from SPS or parsed inline).
    pub short_term_rps: Option<ShortTermRefPicSet>,
}

/// Parse a slice segment header from RBSP data.
///
/// `nal_type` is needed to determine IRAP-specific fields.
/// `sps` and `pps` must be the active parameter sets.
pub fn parse_slice_header(
    rbsp: &[u8],
    nal_type: NalUnitType,
    sps: &Sps,
    pps: &Pps,
) -> DecodeResult<SliceHeader> {
    let mut r = BitstreamReader::new(rbsp);

    let first_slice_segment_in_pic_flag = r.read_flag()?;

    let no_output_of_prior_pics_flag = if nal_type.is_irap() {
        r.read_flag()?
    } else {
        false
    };

    let pps_id = r.read_ue()?;

    let mut dependent_slice_segment_flag = false;
    let mut slice_segment_address = 0u32;

    if !first_slice_segment_in_pic_flag {
        if pps.dependent_slice_segments_enabled_flag {
            dependent_slice_segment_flag = r.read_flag()?;
        }

        let pic_size_in_ctbs = sps_pic_size_in_ctbs(sps);
        let addr_bits = ceil_log2(pic_size_in_ctbs);
        if addr_bits > 0 {
            slice_segment_address = r.read_bits(addr_bits)?;
        }
    }

    // If this is a dependent slice segment, the remaining fields are
    // inherited from the independent slice header.  We stop here.
    if dependent_slice_segment_flag {
        return Ok(SliceHeader {
            first_slice_segment_in_pic_flag,
            no_output_of_prior_pics_flag,
            pps_id,
            dependent_slice_segment_flag,
            slice_segment_address,
            slice_type: 0,
            pic_output_flag: true,
            colour_plane_id: 0,
            pic_order_cnt_lsb: 0,
            short_term_ref_pic_set_idx: 0,
            short_term_ref_pic_set_sps_flag: false,
            short_term_rps: None,
        });
    }

    // Skip num_extra_slice_header_bits.
    for _ in 0..pps.num_extra_slice_header_bits {
        r.read_bit()?;
    }

    let slice_type = r.read_ue()? as u8;

    let pic_output_flag = if pps.output_flag_present_flag {
        r.read_flag()?
    } else {
        true
    };

    let colour_plane_id = if sps.separate_colour_plane_flag {
        r.read_bits(2)? as u8
    } else {
        0
    };

    let mut pic_order_cnt_lsb = 0u32;
    let mut short_term_ref_pic_set_idx = 0u32;
    let mut short_term_ref_pic_set_sps_flag = false;
    let mut short_term_rps: Option<ShortTermRefPicSet> = None;

    if !nal_type.is_idr() {
        pic_order_cnt_lsb = r.read_bits(sps.log2_max_pic_order_cnt_lsb)?;

        short_term_ref_pic_set_sps_flag = r.read_flag()?;

        if !short_term_ref_pic_set_sps_flag {
            // Inline short-term RPS.
            let num_sets = sps.short_term_ref_pic_sets.len();
            let rps = ShortTermRefPicSet::parse(
                &mut r,
                num_sets,
                &sps.short_term_ref_pic_sets,
                num_sets as u32,
            )?;
            short_term_rps = Some(rps);
        } else if !sps.short_term_ref_pic_sets.is_empty() {
            let num_sets = sps.short_term_ref_pic_sets.len() as u32;
            let bits_needed = ceil_log2(num_sets);
            if bits_needed > 0 {
                short_term_ref_pic_set_idx = r.read_bits(bits_needed)?;
            }
            if (short_term_ref_pic_set_idx as usize) < sps.short_term_ref_pic_sets.len() {
                short_term_rps =
                    Some(sps.short_term_ref_pic_sets[short_term_ref_pic_set_idx as usize].clone());
            }
        }

        // We stop parsing here; the remaining slice header fields
        // (long-term refs, temporal MVP, SAO, ref list modification,
        // prediction weights, merge/AMVP, QP delta, etc.) are needed
        // for full decoding but not for the current scaffold.
    }

    Ok(SliceHeader {
        first_slice_segment_in_pic_flag,
        no_output_of_prior_pics_flag,
        pps_id,
        dependent_slice_segment_flag,
        slice_segment_address,
        slice_type,
        pic_output_flag,
        colour_plane_id,
        pic_order_cnt_lsb,
        short_term_ref_pic_set_idx,
        short_term_ref_pic_set_sps_flag,
        short_term_rps,
    })
}

/// Compute `ceil(log2(x))` for `x > 0`.  Returns 0 for `x <= 1`.
fn ceil_log2(x: u32) -> u8 {
    if x <= 1 {
        return 0;
    }
    32 - (x - 1).leading_zeros() as u8
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- NalHeader --

    #[test]
    fn test_nal_header_parse_trail_r() {
        // TRAIL_R (type=1), layer_id=0, temporal_id_plus1=1
        // forbidden=0, type=000001 → byte0 upper7 = 0_000001 = 0x02
        // layer_id=000000, tid_plus1=001 → byte1 = 00000_001 = 0x01
        let data = [0x02, 0x01];
        let hdr = NalHeader::parse(&data).unwrap();
        assert_eq!(hdr.nal_unit_type, 1);
        assert_eq!(hdr.nuh_layer_id, 0);
        assert_eq!(hdr.nuh_temporal_id_plus1, 1);
        assert_eq!(hdr.temporal_id(), 0);
        assert_eq!(hdr.nal_type(), NalUnitType::TrailR);
    }

    #[test]
    fn test_nal_header_parse_idr() {
        // IDR_W_RADL (type=19), layer_id=0, temporal_id_plus1=1
        // type=19=010011, byte0 = 0_010011_0 = 0x26
        // byte1 = 00000_001 = 0x01
        let data = [0x26, 0x01];
        let hdr = NalHeader::parse(&data).unwrap();
        assert_eq!(hdr.nal_unit_type, 19);
        assert_eq!(hdr.nal_type(), NalUnitType::IdrWRadl);
        assert!(hdr.nal_type().is_idr());
        assert!(hdr.nal_type().is_irap());
    }

    #[test]
    fn test_nal_header_parse_vps() {
        // VPS (type=32), layer_id=0, temporal_id_plus1=1
        // type=32=100000, byte0 = 0_100000_0 = 0x40
        // byte1 = 00000_001 = 0x01
        let data = [0x40, 0x01];
        let hdr = NalHeader::parse(&data).unwrap();
        assert_eq!(hdr.nal_unit_type, 32);
        assert_eq!(hdr.nal_type(), NalUnitType::Vps);
        assert!(!hdr.nal_type().is_vcl());
    }

    #[test]
    fn test_nal_header_parse_sps() {
        // SPS (type=33), layer_id=0, temporal_id_plus1=1
        // type=33=100001, byte0 = 0_100001_0 = 0x42
        // byte1 = 00000_001 = 0x01
        let data = [0x42, 0x01];
        let hdr = NalHeader::parse(&data).unwrap();
        assert_eq!(hdr.nal_unit_type, 33);
        assert_eq!(hdr.nal_type(), NalUnitType::Sps);
    }

    #[test]
    fn test_nal_header_parse_pps() {
        // PPS (type=34), layer_id=0, temporal_id_plus1=1
        // type=34=100010, byte0 = 0_100010_0 = 0x44
        // byte1 = 00000_001 = 0x01
        let data = [0x44, 0x01];
        let hdr = NalHeader::parse(&data).unwrap();
        assert_eq!(hdr.nal_unit_type, 34);
        assert_eq!(hdr.nal_type(), NalUnitType::Pps);
    }

    #[test]
    fn test_nal_header_too_short() {
        assert!(NalHeader::parse(&[0x42]).is_err());
        assert!(NalHeader::parse(&[]).is_err());
    }

    // -- NalUnitType --

    #[test]
    fn test_nal_unit_type_roundtrip() {
        for raw in 0..=40u8 {
            let t = NalUnitType::from_raw(raw);
            assert_eq!(t.raw(), raw, "roundtrip failed for raw={raw}");
        }
    }

    #[test]
    fn test_nal_unit_type_is_vcl() {
        assert!(NalUnitType::TrailN.is_vcl());
        assert!(NalUnitType::TrailR.is_vcl());
        assert!(NalUnitType::IdrWRadl.is_vcl());
        assert!(NalUnitType::CraNut.is_vcl());
        assert!(!NalUnitType::Vps.is_vcl());
        assert!(!NalUnitType::Sps.is_vcl());
        assert!(!NalUnitType::Pps.is_vcl());
    }

    #[test]
    fn test_nal_unit_type_is_irap() {
        assert!(NalUnitType::BlaWLp.is_irap());
        assert!(NalUnitType::BlaWRadl.is_irap());
        assert!(NalUnitType::BlaNLp.is_irap());
        assert!(NalUnitType::IdrWRadl.is_irap());
        assert!(NalUnitType::IdrNLp.is_irap());
        assert!(NalUnitType::CraNut.is_irap());
        assert!(!NalUnitType::TrailR.is_irap());
        assert!(!NalUnitType::Sps.is_irap());
    }

    #[test]
    fn test_nal_unit_type_is_idr() {
        assert!(NalUnitType::IdrWRadl.is_idr());
        assert!(NalUnitType::IdrNLp.is_idr());
        assert!(!NalUnitType::CraNut.is_idr());
        assert!(!NalUnitType::BlaWLp.is_idr());
    }

    #[test]
    fn test_nal_unit_type_is_reference() {
        assert!(NalUnitType::TrailR.is_reference()); // type 1, odd
        assert!(!NalUnitType::TrailN.is_reference()); // type 0, even
        assert!(NalUnitType::TsaR.is_reference()); // type 3, odd
        assert!(!NalUnitType::TsaN.is_reference()); // type 2, even
        assert!(NalUnitType::IdrWRadl.is_reference()); // IRAP
        assert!(NalUnitType::CraNut.is_reference()); // IRAP
    }

    #[test]
    fn test_nal_unit_type_display() {
        assert_eq!(NalUnitType::TrailN.to_string(), "TRAIL_N");
        assert_eq!(NalUnitType::IdrWRadl.to_string(), "IDR_W_RADL");
        assert_eq!(NalUnitType::Vps.to_string(), "VPS_NUT");
        assert_eq!(NalUnitType::Sps.to_string(), "SPS_NUT");
        assert_eq!(NalUnitType::Other(42).to_string(), "UNKNOWN(42)");
    }

    #[test]
    fn test_nal_unit_type_is_bla() {
        assert!(NalUnitType::BlaWLp.is_bla());
        assert!(NalUnitType::BlaWRadl.is_bla());
        assert!(NalUnitType::BlaNLp.is_bla());
        assert!(!NalUnitType::IdrWRadl.is_bla());
    }

    #[test]
    fn test_nal_unit_type_is_radl_rasl() {
        assert!(NalUnitType::RadlN.is_radl());
        assert!(NalUnitType::RadlR.is_radl());
        assert!(!NalUnitType::RaslN.is_radl());

        assert!(NalUnitType::RaslN.is_rasl());
        assert!(NalUnitType::RaslR.is_rasl());
        assert!(!NalUnitType::RadlN.is_rasl());
    }

    // -- ceil_log2 --

    #[test]
    fn test_ceil_log2() {
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(4), 2);
        assert_eq!(ceil_log2(5), 3);
        assert_eq!(ceil_log2(8), 3);
        assert_eq!(ceil_log2(9), 4);
        assert_eq!(ceil_log2(16), 4);
        assert_eq!(ceil_log2(17), 5);
        assert_eq!(ceil_log2(64), 6);
        assert_eq!(ceil_log2(100), 7);
    }

    // -- SPS helpers --

    #[test]
    fn test_sps_ctb_helpers() {
        // Minimal synthetic SPS: 1920x1080, min CB size=8 (log2=3), diff=3 → CTB=64
        let sps = Sps {
            vps_id: 0,
            max_sub_layers_minus1: 0,
            temporal_id_nesting_flag: true,
            profile_tier_level: ProfileTierLevel::default(),
            sps_id: 0,
            chroma_format_idc: 1,
            separate_colour_plane_flag: false,
            pic_width_in_luma_samples: 1920,
            pic_height_in_luma_samples: 1080,
            conf_win_left_offset: 0,
            conf_win_right_offset: 0,
            conf_win_top_offset: 0,
            conf_win_bottom_offset: 0,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            log2_max_pic_order_cnt_lsb: 8,
            max_dec_pic_buffering_minus1: vec![4],
            max_num_reorder_pics: vec![2],
            max_latency_increase_plus1: vec![0],
            log2_min_luma_coding_block_size: 3,
            log2_diff_max_min_luma_coding_block_size: 3,
            log2_min_luma_transform_block_size: 2,
            log2_diff_max_min_luma_transform_block_size: 3,
            max_transform_hierarchy_depth_inter: 1,
            max_transform_hierarchy_depth_intra: 1,
            scaling_list_enabled_flag: false,
            scaling_list_data: ScalingListData::default(),
            amp_enabled_flag: true,
            sample_adaptive_offset_enabled_flag: true,
            pcm_enabled_flag: false,
            pcm_sample_bit_depth_luma_minus1: 0,
            pcm_sample_bit_depth_chroma_minus1: 0,
            log2_min_pcm_luma_coding_block_size_minus3: 0,
            log2_diff_max_min_pcm_luma_coding_block_size: 0,
            pcm_loop_filter_disabled_flag: false,
            short_term_ref_pic_sets: vec![],
            long_term_ref_pics_present_flag: false,
            num_long_term_ref_pics_sps: 0,
            lt_ref_pic_poc_lsb_sps: vec![],
            used_by_curr_pic_lt_sps_flag: vec![],
            temporal_mvp_enabled_flag: true,
            strong_intra_smoothing_enabled_flag: true,
        };

        assert_eq!(sps_ctb_size_log2(&sps), 6); // 3+3=6
        assert_eq!(sps_ctb_size(&sps), 64);
        assert_eq!(sps_pic_width_in_ctbs(&sps), 30); // ceil(1920/64)=30
        assert_eq!(sps_pic_height_in_ctbs(&sps), 17); // ceil(1080/64)=17
        assert_eq!(sps_pic_size_in_ctbs(&sps), 510);
        assert_eq!(sps_cropped_width(&sps), 1920);
        assert_eq!(sps_cropped_height(&sps), 1080);
    }

    // -- VpsStore / SpsStore / PpsStore --

    #[test]
    fn test_vps_store() {
        let mut store = VpsStore::new();
        assert!(store.get(0).is_none());

        let vps = Vps {
            vps_id: 0,
            base_layer_internal_flag: true,
            base_layer_available_flag: true,
            max_layers_minus1: 0,
            max_sub_layers_minus1: 0,
            temporal_id_nesting_flag: true,
            profile_tier_level: ProfileTierLevel::default(),
            max_dec_pic_buffering_minus1: vec![4],
            max_num_reorder_pics: vec![0],
            max_latency_increase_plus1: vec![0],
        };
        store.insert(0, vps);
        assert!(store.get(0).is_some());
        assert_eq!(store.get(0).unwrap().vps_id, 0);

        store.clear();
        assert!(store.get(0).is_none());
    }

    #[test]
    fn test_sps_store() {
        let mut store = SpsStore::new();
        assert!(store.get(0).is_none());
        // Inserting and retrieving is tested implicitly via parse_sps tests.
    }

    #[test]
    fn test_pps_store() {
        let mut store = PpsStore::new();
        assert!(store.get(0).is_none());
    }

    // -- ShortTermRefPicSet explicit parsing --

    #[test]
    fn test_st_rps_explicit_simple() {
        // Build RBSP for: num_neg=1, num_pos=0,
        // delta_poc_s0_minus1[0]=0, used_by_curr[0]=1
        // ue(1)=010, ue(0)=1, ue(0)=1, u(1)=1
        // total: 010 1 1 1 = 010_111_xx in a byte = 0b0101_1100 = 0x5C
        let data = [0x5C];
        let mut r = BitstreamReader::new(&data);
        let rps = ShortTermRefPicSet::parse(&mut r, 0, &[], 1).unwrap();
        assert_eq!(rps.num_negative_pics, 1);
        assert_eq!(rps.num_positive_pics, 0);
        assert_eq!(rps.delta_poc_s0.len(), 1);
        assert_eq!(rps.delta_poc_s0[0], -1); // -(0+1) = -1
        assert!(rps.used_by_curr_pic_s0[0]);
    }

    // -- ProfileTierLevel --

    #[test]
    fn test_ptl_parse_basic() {
        // Construct minimal PTL bitstream:
        // general_profile_space=0(2b), tier=0(1b), profile_idc=1(5b) = 0b00_0_00001 = 0x01
        // profile_compat = 0x40000000 (bit 1 set) (32b)
        // progressive=1, interlaced=0, non_packed=1, frame_only=1 (4b) = 0b1011
        // 44 bits of constraint flags (all 0)
        // general_level_idc = 120 (Level 4.0) (8b)
        // No sub-layers.

        let mut bytes = Vec::new();
        // Byte 0: 00_0_00001
        bytes.push(0x01);
        // Bytes 1-4: profile_compat = 0x40000000
        bytes.push(0x40);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.push(0x00);
        // Next nibble: progressive=1, interlaced=0, non_packed=1, frame_only=1 = 0b1011
        // Then 44 bits of zeros, then level_idc=120
        // 4 + 44 = 48 bits = 6 bytes: 0b1011_0000 0x00 0x00 0x00 0x00 0x00
        bytes.push(0xB0);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.push(0x00);
        bytes.push(0x00);
        // level_idc = 120
        bytes.push(120);

        let mut r = BitstreamReader::new(&bytes);
        let ptl = ProfileTierLevel::parse(&mut r, true, 0).unwrap();

        assert_eq!(ptl.general_profile_space, 0);
        assert!(!ptl.general_tier_flag);
        assert_eq!(ptl.general_profile_idc, 1);
        assert!(ptl.general_progressive_source_flag);
        assert!(!ptl.general_interlaced_source_flag);
        assert!(ptl.general_non_packed_constraint_flag);
        assert!(ptl.general_frame_only_constraint_flag);
        assert_eq!(ptl.general_level_idc, 120);
    }
}
