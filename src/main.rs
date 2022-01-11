mod signed_head;
use signed_head::SignedHead;

fn main() {
    let signed_msg = r#"{"head":{"/":"bafybeicyhbhhklw3kdwgrxmf67mhkgjbsjauphsvrzywav63kn7bkpmqfa"},"pubkey":{"/":{"bytes":"CAESIJSklColz5Jq+bVsKPQpxmEwo9avM7y/vVkYSDttBWLI"}},"sig":{"/":{"bytes":"1S4p2vHPXobyPnspQWkCHMjf2n5qQCMb+OehDjUnQbRil3qf95g87VNcIxl6hr66zmhBeJ7h+Y6UnUUhnUMZAQ"}}}"#;
    let signed_head: SignedHead = serde_json::from_str(signed_msg).expect("deser failed");
    let (_pk, head) = signed_head.open().expect("failed to open signed_head");
    println!("head is {:?}", head);
}
