pub fn get_bits<T>(value: u64, start: u8, length: u8) -> T
where
    T: TryFrom<u64>,
    T::Error: std::fmt::Debug,
{
    assert!(start < 64, "Start position must be less than 64");
    assert!(length > 0, "Length must be greater than 0");
    assert!(start + length <= 64, "Start + length must not exceed 64");

    let mask = if length == 64 {
        u64::MAX
    } else {
        (1u64 << length) - 1
    };

    T::try_from((value >> start) & mask).unwrap()
}

pub fn set_bits<T>(value: u64, data: T, start: u8, length: u8) -> u64
where
    u64: From<T>,
{
    assert!(start < 64, "Start position must be less than 64");
    assert!(length > 0, "Length must be greater than 0");
    assert!(start + length <= 64, "Start + length must not exceed 64");

    let data_u64 = u64::from(data);
    let mask = if length == 64 {
        u64::MAX
    } else {
        (1u64 << length) - 1
    };

    (value & !(mask << start)) | ((data_u64 & mask) << start)
}
