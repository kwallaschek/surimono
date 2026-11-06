//! The Rust half of what the window has that no node does: URLs, base64 and
//! text encodings. `glue_platform.js` holds the interfaces; everything here takes
//! and returns strings, so nothing outlives a call but a stream's decoder, which
//! [`Document`](crate::Document) keeps (`decoders`).

use encoding_rs::{DecoderResult, Encoding};
use url::{Url, quirks};

use crate::document::{self, Document};

/// Bytes a URL's text may have, here as for `location`. Chrome takes longer ones.
const MAX: usize = document::MAX_URL_BYTES;

/// URL Standard §6.1 "API URL parser": `input` against `base`, as its
/// serialization, or `None` for a failure and for what is over [`MAX`].
pub(crate) fn url_parse(input: &str, base: Option<&str>) -> Option<String> {
    if input.len() > MAX || base.is_some_and(|b| b.len() > MAX) {
        return None;
    }
    let url = match base {
        Some(b) => Url::parse(b).and_then(|b| b.join(input)),
        None => Url::parse(input),
    };
    url.ok().map(String::from).filter(|u| u.len() <= MAX)
}

/// One getter of §6.1 on the URL serialized as `href`.
pub(crate) fn url_get(href: &str, part: &str) -> String {
    let Ok(url) = Url::parse(href) else {
        return String::new();
    };
    match part {
        "origin" => quirks::origin(&url),
        "protocol" => quirks::protocol(&url).to_owned(),
        "username" => quirks::username(&url).to_owned(),
        "password" => quirks::password(&url).to_owned(),
        "host" => quirks::host(&url).to_owned(),
        "hostname" => quirks::hostname(&url).to_owned(),
        "port" => quirks::port(&url).to_owned(),
        "pathname" => quirks::pathname(&url).to_owned(),
        "search" => quirks::search(&url).to_owned(),
        "hash" => quirks::hash(&url).to_owned(),
        _ => String::new(),
    }
}

/// One setter of §6.1 on the URL serialized as `href`: the new serialization,
/// which is the old one for a value the setter ignores. `None` when the value
/// or the result is over [`MAX`].
pub(crate) fn url_set(href: &str, part: &str, value: &str) -> Option<String> {
    if value.len() > MAX {
        return None;
    }
    let Ok(mut url) = Url::parse(href) else {
        return Some(href.to_owned());
    };
    // A refused value leaves the URL as it was, which is the setter's answer.
    let _ = match part {
        "protocol" => quirks::set_protocol(&mut url, value),
        "username" => quirks::set_username(&mut url, value),
        "password" => quirks::set_password(&mut url, value),
        "host" => quirks::set_host(&mut url, value),
        "hostname" => quirks::set_hostname(&mut url, value),
        "port" => quirks::set_port(&mut url, value),
        "pathname" => {
            quirks::set_pathname(&mut url, value);
            Ok(())
        }
        "search" => {
            quirks::set_search(&mut url, value);
            Ok(())
        }
        "hash" => {
            quirks::set_hash(&mut url, value);
            Ok(())
        }
        _ => Ok(()),
    };
    Some(String::from(url)).filter(|u| u.len() <= MAX)
}

/// URL Standard §5.2's percent-encoding of one name or value.
pub(crate) fn form_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// URL Standard §5.1's decoding of one name or value: `+` is a space, then
/// percent-decoding, then UTF-8 with replacement.
pub(crate) fn form_decode(text: &str) -> String {
    let input = text.as_bytes();
    let hex = |i: usize| input.get(i).and_then(|&b| (b as char).to_digit(16));
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while let Some(&b) = input.get(i) {
        i += 1;
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                if let (Some(h), Some(l)) = (hex(i), hex(i + 1)) {
                    // Two hex digits fit a byte.
                    out.push((h * 16 + l) as u8);
                    i += 2;
                } else {
                    out.push(b);
                }
            }
            _ => out.push(b),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// HTML §8.3 `btoa`: `None` for a character above U+00FF.
pub(crate) fn base64_encode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        bytes.push(u8::try_from(u32::from(c)).ok()?);
    }
    let digit = |x: u32| char::from(BASE64[(x & 63) as usize]);
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let at = |i: usize| chunk.get(i).map_or(0, |&b| u32::from(b));
        let word = at(0) << 16 | at(1) << 8 | at(2);
        out.push(digit(word >> 18));
        out.push(digit(word >> 12));
        out.push(if chunk.len() > 1 {
            digit(word >> 6)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { digit(word) } else { '=' });
    }
    Some(out)
}

/// Infra's "forgiving-base64 decode", for `atob`: the bytes as U+0000 to
/// U+00FF, or `None` for a failure.
pub(crate) fn base64_decode(text: &str) -> Option<String> {
    let mut data: Vec<u8> = text
        .bytes()
        .filter(|b| !matches!(b, b'\t' | b'\n' | 0x0C | b'\r' | b' '))
        .collect();
    if data.len().is_multiple_of(4) {
        for _ in 0..2 {
            if data.last() == Some(&b'=') {
                data.pop();
            }
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = String::with_capacity(data.len() / 4 * 3 + 2);
    let (mut word, mut bits) = (0_u32, 0_u32);
    for b in data {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        word = word << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            // Masked to a byte, which is a char.
            out.push(char::from((word >> bits & 255) as u8));
            word &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Encoding Standard's "get an encoding", as `TextDecoder` names it, or empty
/// for a label it refuses: an unknown one, and `replacement`'s.
pub(crate) fn encoding_name(label: &str) -> String {
    match Encoding::for_label(label.as_bytes()) {
        Some(e) if e != encoding_rs::REPLACEMENT => e.name().to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// `text` holds one byte in each character: what a typed array can give Rust
/// with no `unsafe`, which crates/dom has none of.
fn bytes_of(text: &str) -> Vec<u8> {
    // Each is below 256: the glue makes the string from bytes.
    text.chars().map(|c| c as u8).collect()
}

/// What an open decoder costs against `max_text_bytes`: its memory is outside the
/// JavaScript heap limit, and a script can keep any number open.
const DECODER_COST: usize = 4096;

impl Document {
    /// A new decoder for the encoding `encoding_name` gave, in a free slot, until
    /// its stream ends. Numbered from 1. `None` when the fault is set: each counts
    /// against [`DomLimits::max_text_bytes`](crate::DomLimits::max_text_bytes).
    pub(crate) fn decoder_open(&mut self, name: &str, ignore_bom: bool) -> Option<u32> {
        self.charge_record(DECODER_COST)?;
        let encoding = Encoding::for_label(name.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        let decoder = if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        };
        let slot = self.free_decoders.pop().unwrap_or_else(|| {
            self.decoders.push(None);
            self.decoders.len() as u32
        });
        self.decoders[slot as usize - 1] = Some(decoder);
        Some(slot)
    }

    /// Encoding Standard §7.1 `decode`, one call: the text of `input`'s bytes.
    /// `last` ends the stream and frees the slot. `None` for malformed input under
    /// `fatal`, or for a slot with no decoder. In the middle of a stream the rest of
    /// `input` goes and the decoder keeps its state, as WPT's
    /// `textdecoder-mistakes.any.js` has it, but for one case a byte pushed back
    /// before the error is lost: the decoder's own state doesn't hold it.
    pub(crate) fn decoder_feed(
        &mut self,
        slot: u32,
        input: &str,
        last: bool,
        fatal: bool,
    ) -> Option<String> {
        let at = (slot as usize).checked_sub(1)?;
        let decoder = self.decoders.get_mut(at)?.as_mut()?;
        // encoding_rs forgets a lead byte on empty input (its issue 126).
        if input.is_empty() && !last {
            return Some(String::new());
        }
        let bytes = bytes_of(input);
        let room = if fatal {
            decoder.max_utf8_buffer_length_without_replacement(bytes.len())
        } else {
            decoder.max_utf8_buffer_length(bytes.len())
        }?;
        let mut out = String::with_capacity(room);
        let ok = if fatal {
            let (result, _) = decoder.decode_to_string_without_replacement(&bytes, &mut out, last);
            !matches!(result, DecoderResult::Malformed(..))
        } else {
            let _ = decoder.decode_to_string(&bytes, &mut out, last);
            true
        };
        if last {
            self.decoders[at] = None;
            self.free_decoders.push(slot);
            self.credit_record(DECODER_COST);
        }
        ok.then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_follows_infra() {
        assert_eq!(base64_encode("").as_deref(), Some(""));
        assert_eq!(base64_encode("a").as_deref(), Some("YQ=="));
        assert_eq!(base64_encode("ab").as_deref(), Some("YWI="));
        assert_eq!(base64_encode("abc").as_deref(), Some("YWJj"));
        assert_eq!(base64_encode("\u{ff}").as_deref(), Some("/w=="));
        assert_eq!(base64_encode("\u{100}"), None);
        assert_eq!(base64_decode("YWJj").as_deref(), Some("abc"));
        assert_eq!(base64_decode(" Y W\nI= ").as_deref(), Some("ab"));
        assert_eq!(base64_decode("YQ").as_deref(), Some("a"));
        assert_eq!(base64_decode("/w==").as_deref(), Some("\u{ff}"));
        assert_eq!(base64_decode("Y"), None);
        assert_eq!(base64_decode("Y==="), None);
        assert_eq!(base64_decode("YQ=a"), None);
        assert_eq!(base64_decode("é"), None);
    }

    #[test]
    fn form_decoding_keeps_what_isnt_an_escape() {
        assert_eq!(form_decode("a+b%20c%"), "a b c%");
        assert_eq!(form_decode("%e3%81%82%2"), "あ%2");
        assert_eq!(form_decode("%ff"), "\u{fffd}");
    }

    #[test]
    fn url_text_over_the_cap_is_refused() {
        let long = format!("http://e.test/{}", "a".repeat(MAX));
        assert_eq!(url_parse(&long, None), None);
        assert_eq!(url_parse("/x", Some(&long)), None);
        assert_eq!(url_set("http://e.test/", "hash", &long), None);
    }
}
