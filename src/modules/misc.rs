use crate::object::*;
use std::collections::HashMap;

mod collections;
pub use collections::*;

mod types;
pub use types::*;

mod csv;
pub use csv::*;
mod re;
pub use re::*;

mod struct_heapq;
pub use struct_heapq::*;

mod graphlib;
pub use graphlib::*;

mod weakref;
pub use weakref::*;

mod numbers;
pub use numbers::*;

mod this;
pub use this::*;

mod queue;
pub use queue::*;

mod cmath;
pub use cmath::*;

mod hashlib_extra;
pub use hashlib_extra::*;

mod sysconfig;
pub use sysconfig::*;

mod xml;
pub use xml::*;

mod gettext;
pub use gettext::*;

mod email_utils;
pub use email_utils::*;

mod contextlib;
pub use contextlib::*;

mod getpass;
pub use getpass::*;

mod json_tool;
pub use json_tool::*;

mod logging_config;
pub use logging_config::*;

mod array;
pub use array::*;
mod sunau;
pub use sunau::*;
mod argparse;
pub use argparse::*;

mod gc;
pub use gc::*;
mod locale;
pub use locale::*;
mod colorsys;
pub use colorsys::*;
mod threading;
pub use threading::*;
mod platform;
pub use platform::*;
mod getopt;
pub use getopt::*;

mod email_mime_text;
pub use email_mime_text::*;

mod email_header;
pub use email_header::*;

mod copy;
pub use copy::*;
mod uuid;
pub use uuid::*;
mod ast;
pub use ast::*;

mod email;
pub use email::*;
mod contextvars;
pub use contextvars::*;
mod wave;
pub use wave::*;
mod ssl;
pub use ssl::*;
mod asyncio;
pub use asyncio::*;
mod logging;
pub use logging::*;
mod thread;
pub use thread::*;
mod signal;
pub use signal::*;
mod selectors;
pub use selectors::*;
mod xml_etree;
pub use xml_etree::*;
mod atexit;
pub use atexit::*;

mod pickle_ser;
pub use pickle_ser::*;
mod pickle_de;
pub use pickle_de::*;
mod pickle;
pub use pickle::*;
mod timeit;
pub use timeit::*;
mod configparser;
pub use configparser::*;


// Real Enum/IntEnum/StrEnum/EnumType/auto/unique semantics are implemented
// as real Python source instead — see ENUM_SOURCE (below) and
// VirtualMachine::install_source_defined_stdlib.
pub const ENUM_SOURCE: &str = include_str!("enum_extra.py");
