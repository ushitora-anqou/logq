#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en");

pub mod app;
pub mod filter;
pub mod highlight;
pub mod input;
pub mod render;
