#![allow(dead_code)]
use serde::{de, Deserialize, Deserializer, Serializer};
use std::num;

/// Seralizes a byte string into hex
pub fn as_hex<T, S>(bytes: T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: AsRef<[u8]>,
    S: Serializer,
{
    serializer.serialize_str(&to_hex(bytes.as_ref().to_vec()))
}

/// Creates a Pedersen Commitment from a hex string
pub fn commitment_from_hex<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    String::deserialize(deserializer)
        .and_then(|string| from_hex(string).map_err(|err| Error::custom(err.to_string())))
}

/// Decode a hex string into bytes.
pub fn from_hex(hex_str: String) -> Result<Vec<u8>, num::ParseIntError> {
    if hex_str.len() % 2 == 1 {
        // TODO: other way to instantiate a ParseIntError?
        let err = ("QQQ").parse::<u64>();
        if let Err(e) = err {
            return Err(e);
        }
    }
    let hex_trim = if &hex_str[..2] == "0x" {
        hex_str[2..].to_owned()
    } else {
        hex_str.clone()
    };
    split_n(&hex_trim.trim()[..], 2)
        .iter()
        .map(|b| u8::from_str_radix(b, 16))
        .collect::<Result<Vec<u8>, _>>()
}

fn split_n(s: &str, n: usize) -> Vec<&str> {
    (0..(s.len() - n + 1) / 2 + 1)
        .map(|i| &s[2 * i..2 * i + n])
        .collect()
}

use std::fmt::Write;
pub fn to_hex(bytes: Vec<u8>) -> String {
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:02x}", byte).expect("Unable to write");
    }
    s
}
/// Used to ensure u64s are serialised in json
/// as strings by default, since it can't be guaranteed that consumers
/// will know what to do with u64 literals (e.g. Javascript). However,
/// fields using this tag can be deserialized from literals or strings.
/// From solutions on:
/// https://github.com/serde-rs/json/issues/329
pub mod string_or_u64 {
    use std::fmt;

    use serde::{de, Deserializer, Serializer};

    /// serialize into a string
    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: fmt::Display,
        S: Serializer,
    {
        serializer.collect_str(value)
    }

    /// deserialize from either literal or string
    #[allow(dead_code)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'a> de::Visitor<'a> for Visitor {
            type Value = u64;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(
                    formatter,
                    "a string containing digits or an int fitting into u64"
                )
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(v)
            }
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                s.parse().map_err(de::Error::custom)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// As above, for Options
pub mod opt_string_or_u64 {
    use std::fmt;

    use serde::{de, Deserializer, Serializer};

    /// serialize into string or none
    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: fmt::Display,
        S: Serializer,
    {
        match value {
            Some(v) => serializer.collect_str(v),
            None => serializer.serialize_none(),
        }
    }

    /// deser from 'null', literal or string
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'a> de::Visitor<'a> for Visitor {
            type Value = Option<u64>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(
                    formatter,
                    "null, a string containing digits or an int fitting into u64"
                )
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(Some(v))
            }
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let val: u64 = s.parse().map_err(de::Error::custom)?;
                Ok(Some(val))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
