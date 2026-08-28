use crate::object::*;
use std::collections::HashMap;

mod json;
pub use json::*;

mod collections;
pub use collections::*;

mod functools;
pub use functools::*;

mod itertools;
pub use itertools::*;

mod statistics;
pub use statistics::*;

mod decimal;
pub use decimal::*;
mod decimal_types;
pub use decimal_types::*;
mod decimal_mod;
pub use decimal_mod::*;

mod fractions;
pub use fractions::*;
mod fractions_ops;
pub use fractions_ops::*;
mod fractions_mod;
pub use fractions_mod::*;

mod calendar;
pub use calendar::*;

mod random;
pub use random::*;

use num_traits::ToPrimitive;
use std::rc::Rc;
