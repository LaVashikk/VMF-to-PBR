use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum VmtValue {
    // It's a included block (e.g., replace { ... })
    Block(HashMap<String, VmtValue>),

    // It's a sequence
    Seq(Vec<VmtValue>),

    // It's default key-value string
    Str(String),
}

impl VmtValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            VmtValue::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            VmtValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            VmtValue::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Vmt {
    pub shader: String,
    pub properties: HashMap<String, VmtValue>,
}

impl<'de> Deserialize<'de> for Vmt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VmtVisitor;

        impl<'de> Visitor<'de> for VmtVisitor {
            type Value = Vmt;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a VMT map with a single shader block")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let shader = access
                    .next_key::<String>()?
                    .ok_or_else(|| de::Error::custom("Expected shader name, found empty file"))?;

                let properties = access.next_value::<HashMap<String, VmtValue>>()?;

                Ok(Vmt { shader, properties })
            }
        }

        deserializer.deserialize_map(VmtVisitor)
    }
}
