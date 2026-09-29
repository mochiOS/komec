use kome_native_rt::list::{
    __kome_list_alloc, __kome_list_dealloc, __kome_list_len, __kome_list_release,
    __kome_list_retain,
};

#[test]
fn allocates_and_reference_counts_list_storage() {
    let list = __kome_list_alloc(3);
    unsafe {
        assert_eq!(__kome_list_len(list), 3);
        __kome_list_retain(list);
        assert_eq!(__kome_list_release(list), 0);
        assert_eq!(__kome_list_release(list), 1);
        __kome_list_dealloc(list);
    }
}
