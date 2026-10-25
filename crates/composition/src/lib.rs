#![no_std]
//! Remaining in-repo composition values: Unit declarations, deployments and image targets.

pub use unit;

pub mod deployments;
pub mod image_builds;
pub mod units;

pub use deployments::DEPLOYMENTS;
pub use image_builds::IMAGE_BUILDS;
pub use units::PROGRAMS;
