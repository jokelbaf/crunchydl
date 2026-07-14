use std::num::NonZeroU32;
use std::time::Duration;

use crate::Error;

/// A media timestamp scale expressed as ticks per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeBase(NonZeroU32);

impl TimeBase {
    /// Construct a nonzero media time base.
    ///
    /// # Errors
    ///
    /// Returns an error when `ticks_per_second` is zero.
    pub fn new(ticks_per_second: u32) -> Result<Self, Error> {
        NonZeroU32::new(ticks_per_second)
            .map(Self)
            .ok_or(Error::Invalid("zero media timescale"))
    }

    /// Return the number of ticks per second.
    #[must_use]
    pub fn ticks_per_second(self) -> u32 {
        self.0.get()
    }

    /// Convert unsigned ticks to a duration without floating-point rounding.
    ///
    /// # Errors
    ///
    /// Returns an error if nanosecond conversion overflows.
    pub fn duration(self, ticks: u64) -> Result<Duration, Error> {
        let nanos = u128::from(ticks)
            .checked_mul(1_000_000_000)
            .ok_or(Error::Overflow("duration"))?
            / u128::from(self.0.get());
        let nanos = u64::try_from(nanos).map_err(|_| Error::Overflow("duration"))?;
        Ok(Duration::from_nanos(nanos))
    }
}

/// A signed timestamp in one track's media time base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    ticks: i64,
    time_base: TimeBase,
}

impl Timestamp {
    /// Construct a timestamp from signed ticks and a time base.
    #[must_use]
    pub fn new(ticks: i64, time_base: TimeBase) -> Self {
        Self { ticks, time_base }
    }

    /// Return the signed media ticks.
    #[must_use]
    pub fn ticks(self) -> i64 {
        self.ticks
    }

    /// Return the timestamp time base.
    #[must_use]
    pub fn time_base(self) -> TimeBase {
        self.time_base
    }
}
