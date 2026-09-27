use ceiling_limiter::CeilingLimiter;
use nih_plug::prelude::*;

fn main() {
    nih_export_standalone::<CeilingLimiter>();
}
