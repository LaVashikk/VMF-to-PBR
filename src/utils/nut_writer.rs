use serde_json::Value;

pub fn value_to_squirrel(value: &Value, indent: usize) -> String {
    let tabs = "\t".repeat(indent);

    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(s) => {
            if s.starts_with("__VECTOR__") {
                s.replace("__VECTOR__", "Vector")
            } else {
                format!("\"{}\"", s)
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() { return "[]".to_string(); }
            let mut out = String::from("[\n");
            for (i, v) in arr.iter().enumerate() {
                out.push_str(&format!("{}\t{}", tabs, value_to_squirrel(v, indent + 1)));
                if i < arr.len() - 1 { out.push(','); }
                out.push('\n');
            }
            out.push_str(&format!("{}]", tabs));
            out
        }
        Value::Object(obj) => {
            if obj.is_empty() { return "{}".to_string(); }
            let mut out = String::from("{\n");
            for (i, (k, v)) in obj.iter().enumerate() {
                out.push_str(&format!("{}\t{} = {}", tabs, k, value_to_squirrel(v, indent + 1)));
                if i < obj.len() - 1 { out.push(','); }
                out.push('\n');
            }
            out.push_str(&format!("{}}}", tabs));
            out
        }
    }
}
