//! A system helper that raises the CPU energy-performance preference while
//! desktop motion starts, then puts the previous value back.
//!
//! Requests come over the system bus from the active local session (polkit
//! action [`service::ACTION`]). Hosts without an EPP interface, with a
//! performance or power-saver choice in effect, or with a daemon that rewrites
//! the preference continuously are left unchanged.

pub mod boost;
pub mod cpu;
pub mod service;
