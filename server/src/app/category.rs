use crate::app::{CategoryId, MaybeLoaded};
use crate::app::community::Community;

pub struct Category {
    pub id: CategoryId,
    pub community: MaybeLoaded<Community>,
    pub name: String,
    pub sort_index: u32,
}