//! Build-selected model defaults for Noctum Micro firmware and benches.
//!
//! Filter / voice / rate choices come from Cargo features (see Makefile).
//! Call sites should read these constants only.

use synth_core::dsp::{FilterOversampling, FilterType};

#[cfg(not(any(
    feature = "filter-distributed-newton",
    feature = "filter-scalar-feedback",
    feature = "filter-gain-limited",
    feature = "filter-huovilainen",
    feature = "filter-cascaded-svf",
)))]
compile_error!(
    "enable exactly one filter feature: filter-distributed-newton, \
     filter-scalar-feedback, filter-gain-limited, filter-huovilainen, \
     or filter-cascaded-svf"
);

#[cfg(feature = "filter-distributed-newton")]
pub const FILTER_TYPE: FilterType = FilterType::DistributedNewtonTpt;
#[cfg(feature = "filter-scalar-feedback")]
pub const FILTER_TYPE: FilterType = FilterType::ScalarFeedbackTpt;
#[cfg(feature = "filter-gain-limited")]
pub const FILTER_TYPE: FilterType = FilterType::GainLimitedTpt;
#[cfg(feature = "filter-huovilainen")]
pub const FILTER_TYPE: FilterType = FilterType::HuovilainenLadder;
#[cfg(feature = "filter-cascaded-svf")]
pub const FILTER_TYPE: FilterType = FilterType::CascadedTptSvf;

pub const FILTER_OVERSAMPLING: FilterOversampling = FilterOversampling::Off;
