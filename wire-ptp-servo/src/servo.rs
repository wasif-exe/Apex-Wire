#[derive(Debug, Clone, Copy)]
pub struct PiServoConfig {
    pub kp: f64,
    pub ki: f64,
    pub max_frequency_ppm: f64,
}
impl Default for PiServoConfig {
    fn default() -> Self {
        Self {
            kp: 0.7,
            ki: 0.3,
            max_frequency_ppm: 100.0,
        }
    }
}
pub struct PiServoController {
    pub config: PiServoConfig,
    pub integral_err_ns: f64,
    pub freq_adj_ppm: f64,
    pub phase_error_ps: f64,
    pub state_initialized: bool,
}
impl PiServoController {
    pub fn new(config: PiServoConfig) -> Self {
        Self {
            config,
            integral_err_ns: 0.0,
            freq_adj_ppm: 0.0,
            phase_error_ps: 0.0,
            state_initialized: false,
        }
    }
    #[inline(always)]
    pub fn sample_ptp_offset(&mut self, offset_ns: f64, delta_t_sec: f64) -> f64 {
        if !self.state_initialized {
            self.state_initialized = true;
            self.phase_error_ps = offset_ns * 1000.0;
        }
        self.phase_error_ps = offset_ns * 1000.0;
        self.integral_err_ns += offset_ns * delta_t_sec;
        let p_out = self.config.kp * offset_ns;
        let i_out = self.config.ki * self.integral_err_ns;
        let total_adj = (p_out + i_out).clamp(-self.config.max_frequency_ppm, self.config.max_frequency_ppm);
        self.freq_adj_ppm = total_adj;
        total_adj
    }
    pub fn apply_kernel_ptp_adj(&self, ptp_fd: libc::c_int) -> bool {
        if ptp_fd < 0 {
            return false;
        }
        #[repr(C)]
        struct Timex {
            modes: u32,
            offset: i64,
            freq: i64,
            maxerror: i64,
            esterror: i64,
            status: i32,
            constant: i64,
            precision: i64,
            tolerance: i64,
            time: libc::timeval,
            tick: i64,
            ppsfreq: i64,
            jitter: i64,
            stbcnt: i64,
            actime: i64,
            calib: i64,
            error: i64,
            _pad: [i32; 12],
        }
        // SAFETY: Invoking clock_adjtime syscall for hardware PTP clock drift tuning.
        unsafe {
            let mut tx: Timex = std::mem::zeroed();
            tx.modes = 0x0002;
            tx.freq = (self.freq_adj_ppm * 65536.0) as i64;
            let ret = libc::syscall(libc::SYS_clock_adjtime, 3, &mut tx as *mut _);
            ret == 0
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_ptp_pi_servo() {
        let mut servo = PiServoController::new(PiServoConfig::default());
        let adj1 = servo.sample_ptp_offset(1.0, 1.0);
        assert!(adj1 > 0.0);
        let adj2 = servo.sample_ptp_offset(0.1, 1.0);
        assert!(adj2 < adj1);
        assert!(servo.phase_error_ps.abs() < 500.0);
    }
}
