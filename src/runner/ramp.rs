use crate::core::config::Config;

/// Calculate the current target RPS at a given elapsed time.
///
/// Implements a linear ramp profile:
/// - Ramp-up: linearly increases from 0 to target_rps
/// - Steady state: constant target_rps
/// - Ramp-down: linearly decreases from target_rps to 0
pub fn current_rps(cfg: &Config, elapsed_secs: f64) -> f64 {
    let total_ramp_secs = cfg.ramp_up_secs as f64;

    // Ramp-up phase
    if elapsed_secs < total_ramp_secs {
        if cfg.ramp_up_secs == 0 {
            return cfg.target_rps as f64;
        }
        return cfg.target_rps as f64 * (elapsed_secs / total_ramp_secs);
    }

    // Steady state
    let steady_end = (cfg.ramp_up_secs + cfg.steady_dur_secs) as f64;
    if steady_end == 0.0 {
        // No steady phase, go to ramp-down
    } else if elapsed_secs < steady_end {
        return cfg.target_rps as f64;
    }

    // Ramp-down phase
    let total_dur = (cfg.ramp_up_secs + cfg.steady_dur_secs + cfg.ramp_down_secs) as f64;
    if elapsed_secs < total_dur {
        if cfg.ramp_down_secs == 0 {
            return 0.0;
        }
        let remaining = total_dur - elapsed_secs;
        return cfg.target_rps as f64 * (remaining / cfg.ramp_down_secs as f64);
    }

    // After test duration
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(ramp_up: u64, steady: u64, ramp_down: u64, target: u32) -> Config {
        Config {
            url: "http://localhost".into(),
            target_rps: target,
            ramp_up_secs: ramp_up,
            steady_dur_secs: steady,
            ramp_down_secs: ramp_down,
            ..Default::default()
        }
    }

    #[test]
    fn test_no_ramp_immediate_full_rps() {
        let c = cfg(0, 10, 0, 100);
        assert_eq!(current_rps(&c, 0.0), 100.0);
        assert_eq!(current_rps(&c, 5.0), 100.0);
        assert_eq!(current_rps(&c, 9.9), 100.0);
    }

    #[test]
    fn test_ramp_up_linear() {
        let c = cfg(10, 10, 0, 100);
        let eps = 0.1;
        assert!((current_rps(&c, 5.0) - 50.0).abs() < eps);
        assert!((current_rps(&c, 10.0) - 100.0).abs() < eps);
    }

    #[test]
    fn test_steady_state() {
        let c = cfg(5, 20, 5, 200);
        assert_eq!(current_rps(&c, 5.0), 200.0);
        assert_eq!(current_rps(&c, 15.0), 200.0);
        assert_eq!(current_rps(&c, 24.9), 200.0);
    }

    #[test]
    fn test_ramp_down_linear() {
        let c = cfg(0, 10, 10, 100);
        let eps = 0.1;
        assert!((current_rps(&c, 15.0) - 50.0).abs() < eps);
        assert!(current_rps(&c, 19.9) < 10.0);
        assert_eq!(current_rps(&c, 20.0), 0.0);
    }

    #[test]
    fn test_full_ramp_profile() {
        let c = cfg(10, 20, 10, 100);
        let eps = 0.5;
        // Start: 0
        assert!(current_rps(&c, 0.0) < 1.0);
        // Mid ramp-up
        assert!((current_rps(&c, 5.0) - 50.0).abs() < eps);
        // Steady
        assert!((current_rps(&c, 15.0) - 100.0).abs() < eps);
        // Ramp-down
        assert!((current_rps(&c, 35.0) - 50.0).abs() < eps);
        // End
        assert_eq!(current_rps(&c, 40.0), 0.0);
        // Past end
        assert_eq!(current_rps(&c, 50.0), 0.0);
    }
}
