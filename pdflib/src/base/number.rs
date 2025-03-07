/// A PDF number, which can be integer or real.
///
/// The specification does not require particular bit widths, so `i64` and `f64` were chosen,
/// respectively.
///
/// NB that values with a decimal dot will be parsed as [`Number::Real`], even if they have no 
/// decimal part.
#[derive(Debug, PartialEq, Clone)]
pub enum Number {
    Int(i64),
    Real(f64)
}
