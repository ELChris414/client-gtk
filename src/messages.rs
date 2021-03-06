use chrono::prelude::*;
use pulldown_cmark::{html as md_html, Parser as MDParser};
use std::collections::HashMap;
use std::fmt::Write;
use synac::common::Message;

pub struct Messages {
    messages: HashMap<usize, Vec<Message>>
}
impl Messages {
    pub fn new() -> Self {
        Messages {
            messages: HashMap::new()
        }
    }
    pub fn add(&mut self, msg: Message) {
        let messages = self.messages.entry(msg.channel).or_insert_with(Vec::default);
        let i = match messages.binary_search_by_key(&msg.timestamp, |msg| msg.timestamp) {
            Err(i) => i,
            Ok(mut i) => {
                let original_timestamp = Some(messages[i].timestamp);
                while i > 0 && messages.get(i-1).map(|msg| msg.timestamp) == original_timestamp {
                    i -= 1;
                }
                loop {
                    let message = messages.get_mut(i);
                    if message.as_ref().map(|msg| msg.id) == Some(msg.id) {
                        *message.unwrap() = msg;
                        return;
                    }
                    if message.map(|msg| msg.timestamp) != original_timestamp {
                        break;
                    }
                    i += 1
                }
                i
            }
        };
        messages.insert(i, msg);
    }
    pub fn remove(&mut self, id: usize) -> Option<usize> {
        for (channel, messages) in &mut self.messages {
            if let Some(i) = messages.iter().position(|msg| msg.id == id) {
                messages.remove(i);
                return Some(*channel);
            }
        }
        None
    }
    pub fn get(&self, channel: usize) -> &[Message] {
        self.messages.get(&channel).map(|inner| &*inner as &[Message]).unwrap_or(&[])
    }
    pub fn has(&self, channel: usize) -> bool {
        self.messages.contains_key(&channel)
    }
}

pub fn format_timestamp(output: &mut String, timestamp: i64) {
    let time  = Utc.timestamp(timestamp, 0);
    let local = time.with_timezone(&Local);
    let now   = Local::now();

    match now.num_days_from_ce() - local.num_days_from_ce() {
        0 => output.push_str("Today"),
        1 => output.push_str("Yesterday"),
        2 => output.push_str("Two days ago"),
        3 => output.push_str("Three days ago"),
        x if x < 7 => output.push_str("A few days ago"),
        7 => output.push_str("A week ago"),
        _ => {
            write!(output, "{}-{}-{}", local.year(), local.month(), local.day()).unwrap();
        }
    }
    output.push_str(" at ");
    let (is_pm, hour) = local.hour12();
    write!(output, "{}:{:02} ", hour, local.minute()).unwrap();
    output.push_str(if is_pm { "PM" } else { "AM" });
}
pub fn markdown(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    md_html::push_html(&mut output, MDParser::new(&input));

    output
}
