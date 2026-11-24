//! Allocation-free conservative JSON budgets, with exact fallback at a limit.

use serde::Serialize;
use serde::ser::{self, SerializeMap, SerializeSeq, SerializeStruct};
use std::{fmt, io};

pub(super) fn size(value: &impl Serialize, limit: usize) -> usize {
    let mut count = Counter(0);

    match value.serialize(&mut count) {
        Ok(()) if count.0 <= limit => count.0,
        _ => {
            let mut writer = Exact { bytes: 0, limit };

            if serde_json::to_writer(&mut writer, value).is_ok() {
                writer.bytes
            } else {
                limit.saturating_add(1)
            }
        }
    }
}

struct Exact {
    bytes: usize,
    limit: usize,
}

impl io::Write for Exact {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());

        if self.bytes > self.limit {
            return Err(io::Error::other("inspection byte limit"));
        }

        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Counter(usize);

impl Counter {
    fn add(&mut self, bytes: usize) {
        self.0 = self.0.saturating_add(bytes);
    }

    fn string(&mut self, value: &str) {
        self.add(value.len().saturating_add(2));

        for byte in value.bytes() {
            self.add(match byte {
                b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 1,
                0..=31 => 5,
                _ => 0,
            });
        }
    }

    fn unsigned(&mut self, value: u128) {
        self.add(value.checked_ilog10().map_or(1, |n| n as usize + 1));
    }
}

macro_rules! integers {
    ($($method:ident($ty:ty)),*) => {$(
        fn $method(self, value: $ty) -> Result<(), Self::Error> {
            self.unsigned(value as u128);
            Ok(())
        }
    )*};
}

macro_rules! signed {
    ($($method:ident($ty:ty)),*) => {$(
        fn $method(self, value: $ty) -> Result<(), Self::Error> {
            self.add(usize::from(value < 0));
            self.unsigned(value.unsigned_abs() as u128);
            Ok(())
        }
    )*};
}

impl<'a> ser::Serializer for &'a mut Counter {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;

    integers!(
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128)
    );

    signed!(
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128)
    );

    fn serialize_bool(self, value: bool) -> Result<(), Self::Error> {
        self.add(if value { 4 } else { 5 });

        Ok(())
    }

    // JSON's finite binary floats require at most 32 bytes; nonfinite values become null.
    fn serialize_f32(self, _: f32) -> Result<(), Self::Error> {
        self.add(32);

        Ok(())
    }

    fn serialize_f64(self, _: f64) -> Result<(), Self::Error> {
        self.add(32);

        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<(), Self::Error> {
        self.serialize_str(value.encode_utf8(&mut [0; 4]))
    }

    fn serialize_str(self, value: &str) -> Result<(), Self::Error> {
        self.string(value);

        Ok(())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), Self::Error> {
        self.add(value.len().saturating_mul(4).saturating_add(2));

        Ok(())
    }

    fn serialize_none(self) -> Result<(), Self::Error> {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.add(4);

        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.string(variant);
        self.add(3);

        value.serialize(self)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.add(2);

        Ok(Compound(self))
    }

    fn serialize_tuple(self, len: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.string(variant);
        self.add(3);

        self.serialize_seq(Some(len))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.add(2);

        Ok(Compound(self))
    }

    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.string(variant);
        self.add(3);

        self.serialize_map(Some(len))
    }

    fn collect_str<T: fmt::Display + ?Sized>(self, value: &T) -> Result<(), Self::Error> {
        struct DisplayCounter<'a>(&'a mut Counter);

        impl fmt::Write for DisplayCounter<'_> {
            fn write_str(&mut self, value: &str) -> fmt::Result {
                self.0.string(value);
                self.0.0 = self.0.0.saturating_sub(2);

                Ok(())
            }
        }
        self.add(2);

        fmt::write(&mut DisplayCounter(self), format_args!("{value}")).map_err(ser::Error::custom)
    }
}

struct Compound<'a>(&'a mut Counter);

impl SerializeSeq for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.0.add(1);

        value.serialize(&mut *self.0)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

macro_rules! tuples {
    ($($trait:ident),*) => {$(
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = serde_json::Error;
            fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> { SerializeSeq::serialize_element(self, value) }
            fn end(self) -> Result<(), Self::Error> { Ok(()) }
        }
    )*};
}

impl ser::SerializeTuple for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

tuples!(SerializeTupleStruct, SerializeTupleVariant);

impl SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.0.add(4);

        key.serialize(&mut *self.0)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut *self.0)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SerializeStruct for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.0.string(key);
        self.0.add(2);

        value.serialize(&mut *self.0)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    enum Fixture {
        Unit,
        Newtype(String),
        Tuple(i64, f64),
        Struct { text: String, bytes: Vec<u8> },
    }

    #[test]
    fn conservative_bounds_and_exact_limit_fallback_preserve_escaped_payloads() {
        for value in [
            Fixture::Unit,
            Fixture::Newtype("\\\"\n\0é🦀".into()),
            Fixture::Tuple(i64::MIN, f64::MIN_POSITIVE),
            Fixture::Struct {
                text: "x".repeat(100),
                bytes: vec![0, 1, 255],
            },
        ] {
            let actual = serde_json::to_vec(&value).unwrap().len();

            assert!(size(&value, usize::MAX) >= actual);
            assert_eq!(size(&value, actual), actual);
            assert!(size(&value, actual - 1) > actual - 1);
        }

        for value in [
            serde_json::json!({"\n":"\\\"\0é", "float":-1.2345e100}),
            serde_json::json!([null, true, false, {}, [], 1, -10]),
        ] {
            let actual = serde_json::to_vec(&value).unwrap().len();

            assert!(size(&value, usize::MAX) >= actual);
            assert_eq!(size(&value, actual), actual);
        }
    }

    #[test]
    fn bounds_never_undercount_nested_values_numeric_keys_or_utf8_escapes() {
        let escaped: String = (0..=127).map(|v| char::from_u32(v).unwrap()).collect();

        for count in 0..100 {
            let fields: std::collections::BTreeMap<_, _> = (0..count)
                .map(|i| {
                    (
                        i,
                        serde_json::json!({"text":format!("{escaped}é🦀{i}"), "number":-1.234e-100,
                    "nested":[null,true,false, i, {"quoted\\\"":[]}]}),
                    )
                })
                .collect();

            let actual = serde_json::to_vec(&fields).unwrap().len();

            assert!(size(&fields, usize::MAX) >= actual);
            assert_eq!(size(&fields, actual), actual);
            assert!(size(&fields, actual.saturating_sub(1)) >= actual);
        }
    }
}
