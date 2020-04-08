use crate::types;


#[inline]
pub fn event_from_i32(i: i32) -> types::AnnounceEvent {
    match i {
        1 => types::AnnounceEvent::Completed,
        2 => types::AnnounceEvent::Started,
        3 => types::AnnounceEvent::Stopped,
        _ => types::AnnounceEvent::None
    }
}


#[inline]
pub fn event_to_i32(event: types::AnnounceEvent) -> i32 {
    match event {
        types::AnnounceEvent::None => 0,
        types::AnnounceEvent::Completed => 1,
        types::AnnounceEvent::Started => 2,
        types::AnnounceEvent::Stopped => 3
    }
}