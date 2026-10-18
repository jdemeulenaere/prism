//! The target the Tier 1 backend generates code for: the host, with fixed settings (D2 of
//! `codegen-core`).

use crate::error::BackendError;
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};

/// The Cranelift settings of every Tier 1 compilation, as `(name, value)` pairs.
pub fn settings() -> Vec<(&'static str, &'static str)> {
    vec![
        ("opt_level", "speed"),
        // The IR verifier checks every function in debug builds of `prism`, which run the tests;
        // release builds skip it for compile time.
        (
            "enable_verifier",
            if cfg!(debug_assertions) {
                "true"
            } else {
                "false"
            },
        ),
        // Profilers, and later backtraces, walk frame pointers.
        ("preserve_frame_pointers", "true"),
        // A large frame always touches the guard page, so a stack overflow faults there.
        ("enable_probestack", "true"),
        ("probestack_strategy", "inline"),
        // `cranelift-jit` needs long-range calls of runtime functions, and position-independent
        // code exactly on x86-64, where it resolves symbols through a GOT; it rejects PIC on
        // every other architecture.
        ("use_colocated_libcalls", "false"),
        (
            "is_pic",
            if cfg!(target_arch = "x86_64") {
                "true"
            } else {
                "false"
            },
        ),
    ]
}

/// The host ISA with [`settings`].
pub fn isa() -> Result<OwnedTargetIsa, BackendError> {
    let mut flags = settings::builder();
    for (name, value) in settings() {
        flags
            .set(name, value)
            .map_err(|error| BackendError::new(format!("setting {}: {}", name, error)))?;
    }
    let builder = cranelift_native::builder()
        .map_err(|message| BackendError::new(format!("unsupported host: {}", message)))?;
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|error| BackendError::new(format!("building the host ISA: {}", error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_isa_has_the_fixed_settings() {
        let isa = isa().expect("the host is supported");
        let flags = isa.flags();
        assert_eq!(flags.opt_level(), settings::OptLevel::Speed);
        assert!(flags.preserve_frame_pointers());
        assert!(flags.enable_probestack());
        assert_eq!(flags.is_pic(), cfg!(target_arch = "x86_64"));
        assert!(!flags.use_colocated_libcalls());
    }
}
