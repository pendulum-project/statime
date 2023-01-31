#![no_main]

use libfuzzer_sys::fuzz_target;

use statime::datastructures::messages::Message;

fuzz_target!(|data: &[u8]| {
    let message = Message::deserialize(data);

    match message {
        Ok(message) => {
            message.serialize_vec();
        },
        Err(_) => (),
    }
});
