use uuid::Uuid;

pub fn new_op_id() -> String {
    format!("avc_op_{}", Uuid::new_v4())
}

pub fn new_change_id() -> String {
    format!("avc_ch_{}", Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_id_format() {
        let id = new_op_id();
        assert!(id.starts_with("avc_op_"));
        assert_eq!(id.len(), 7 + 36); // prefix + uuid
    }

    #[test]
    fn change_id_format() {
        let id = new_change_id();
        assert!(id.starts_with("avc_ch_"));
        assert_eq!(id.len(), 7 + 36);
    }
}
