use crate::types::AnnounceEvent;


#[inline]
pub fn event_from_i32(i: i32) -> AnnounceEvent {
    match i {
        1 => AnnounceEvent::Completed,
        2 => AnnounceEvent::Started,
        3 => AnnounceEvent::Stopped,
        _ => AnnounceEvent::None
    }
}


#[inline]
pub fn event_to_i32(event: AnnounceEvent) -> i32 {
    match event {
        AnnounceEvent::None => 0,
        AnnounceEvent::Completed => 1,
        AnnounceEvent::Started => 2,
        AnnounceEvent::Stopped => 3
    }
}