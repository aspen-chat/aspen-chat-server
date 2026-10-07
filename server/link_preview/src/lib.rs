//! Reading what a link in a message shows (`app::link_preview`): fetching a page within bounds,
//! through `aspen_outbound`, and reading its metadata (`html_meta`), with what Reddit's posts
//! (`reddit`) and the video providers whose players are embedded (`video`) need besides. What is
//! stored, announced, and to whom is the server's.

pub mod fetch;
pub mod html_meta;
mod reddit;
mod video;
