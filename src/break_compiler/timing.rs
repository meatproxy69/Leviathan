//! Exact bounded rational source/grid positions, encoded into the existing
//! row+f64-delay model without changing its evaluator or legacy project clock.
use crate::RATE;
#[derive(Clone, Copy)]
pub(crate) struct Rational {
    n: u128,
    d: u128,
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}
fn overflow() -> String {
    "Compiler rational timing exceeds its bounded integer range".into()
}
impl Rational {
    pub(crate) fn new(n: u128, d: u128) -> Self {
        debug_assert!(d > 0);
        let g = gcd(n, d);
        Self { n: n / g, d: d / g }
    }
    pub(crate) fn bpm(value: f64) -> Self {
        debug_assert!(value.is_finite() && (20.0..=400.0).contains(&value));
        let bits = value.to_bits();
        let significand = u128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52));
        let shift = 1075 - ((bits >> 52) & 0x7ff);
        Self::new(significand, 1u128 << shift)
    }
    pub(crate) fn from_nonnegative_f64(value: f64) -> Result<Self, String> {
        if !value.is_finite() || value < 0.0 {
            return Err(overflow());
        }
        if value == 0.0 {
            return Ok(Self::new(0, 1));
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let significand =
            u128::from((bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 });
        let power = if exponent == 0 {
            -1074
        } else {
            exponent - 1075
        };
        if power >= 0 {
            Ok(Self::new(
                significand
                    .checked_mul(1u128.checked_shl(power as u32).ok_or_else(overflow)?)
                    .ok_or_else(overflow)?,
                1,
            ))
        } else {
            let mut numerator = significand;
            let mut shift = (-power) as u32;
            let cancel = numerator.trailing_zeros().min(shift);
            numerator >>= cancel;
            shift -= cancel;
            Ok(Self::new(
                numerator,
                1u128.checked_shl(shift).ok_or_else(overflow)?,
            ))
        }
    }
    pub(crate) fn floor(self) -> Result<u64, String> {
        u64::try_from(self.n / self.d).map_err(|_| overflow())
    }
    pub(crate) fn sub(self, other: Self) -> Result<Self, String> {
        let g = gcd(self.d, other.d);
        let a = self.n.checked_mul(other.d / g).ok_or_else(overflow)?;
        let b = other.n.checked_mul(self.d / g).ok_or_else(overflow)?;
        Ok(Self::new(
            a.checked_sub(b).ok_or_else(overflow)?,
            self.d.checked_mul(other.d / g).ok_or_else(overflow)?,
        ))
    }
    pub(crate) fn add(self, other: Self) -> Result<Self, String> {
        let g = gcd(self.d, other.d);
        let a = self.n.checked_mul(other.d / g).ok_or_else(overflow)?;
        let b = other.n.checked_mul(self.d / g).ok_or_else(overflow)?;
        Ok(Self::new(
            a.checked_add(b).ok_or_else(overflow)?,
            self.d.checked_mul(other.d / g).ok_or_else(overflow)?,
        ))
    }
    pub(crate) fn mul(self, other: Self) -> Result<Self, String> {
        let a = gcd(self.n, other.d);
        let b = gcd(other.n, self.d);
        Ok(Self::new(
            (self.n / a).checked_mul(other.n / b).ok_or_else(overflow)?,
            (self.d / b).checked_mul(other.d / a).ok_or_else(overflow)?,
        ))
    }
    pub(crate) fn scale(self, n: u64) -> Result<Self, String> {
        self.mul(Self::new(u128::from(n), 1))
    }
    pub(crate) fn div(self, other: Self) -> Result<Self, String> {
        if other.n == 0 {
            return Err(overflow());
        }
        self.mul(Self::new(other.d, other.n))
    }
    pub(crate) fn less(self, other: Self) -> Result<bool, String> {
        let g = gcd(self.d, other.d);
        Ok(self.n.checked_mul(other.d / g).ok_or_else(overflow)?
            < other.n.checked_mul(self.d / g).ok_or_else(overflow)?)
    }
    pub(crate) fn as_f64(self) -> f64 {
        self.n as f64 / self.d as f64
    }
    pub(crate) fn ceil(self) -> Result<u64, String> {
        u64::try_from(self.n / self.d + u128::from(!self.n.is_multiple_of(self.d)))
            .map_err(|_| overflow())
    }
    pub(crate) fn round(self) -> Result<u64, String> {
        let remainder = self.n % self.d;
        u64::try_from(self.n / self.d + u128::from(remainder >= self.d - remainder))
            .map_err(|_| overflow())
    }
}
/// Prefer the nearest row/delay. At a half-frame tie, move at most a few f64
/// ULPs to the side that represents the exact rational timestamp in the current
/// evaluator. If it cannot be represented inside this pattern, reject instead
/// of silently producing a different frame. No arbitrary timing epsilon.
pub(crate) fn encode(
    beat: Rational,
    prefix: u64,
    lpb: u16,
    per_row: f64,
    bpm: Rational,
) -> Result<(u32, f64, u64), String> {
    let units = beat.scale(u64::from(lpb))?;
    let row = u32::try_from(units.n / units.d).map_err(|_| overflow())?;
    let delay = (units.n % units.d) as f64 / units.d as f64;
    let frame = Rational::new(u128::from(prefix), u128::from(lpb))
        .add(beat)?
        .scale(u64::from(RATE) * 60)?
        .div(bpm)?
        .round()?;
    let mut absolute = (prefix + u64::from(row)) as f64 + delay;
    for _ in 0..32 {
        let relative = absolute - prefix as f64;
        if relative < 0.0 || relative >= 65536.0 {
            return Err("Exact compiler timestamp cannot be encoded inside this pattern; change tempo or LPB".into());
        }
        let row = relative.floor() as u32;
        let delay = relative - f64::from(row);
        let encoded = (prefix + u64::from(row)) as f64 + delay;
        let actual = (encoded * per_row).round() as u64;
        if actual == frame {
            return Ok((row, delay, frame));
        }
        absolute = if actual < frame {
            encoded.next_up()
        } else {
            encoded.next_down()
        };
    }
    Err("Exact compiler timestamp cannot be represented by row/delay at this tempo and LPB".into())
}
