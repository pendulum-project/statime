#![no_main]

use libfuzzer_sys::fuzz_target;

use statime::datastructures::messages::Message;

fuzz_target!(|data: &[u8]| {
    let _message = Message::deserialize(data);
});
