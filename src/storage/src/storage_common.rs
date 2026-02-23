#[cfg(test)]
mod tests {
    use common::physical::config::ServerConfig;
    use common::testutil::{gen_random_int, get_random_byte_vec, get_rng};
    use rstest::rstest;

    use common::ids::{ContainerId, Permissions, TransactionId};
    use common::traits::storage_trait::StorageTrait;

    use heapstore::storage_manager::StorageManager as HeapStorageManager;
    use memstore::storage_manager::StorageManager as MemStorageManager;
    use sledstore::storage_manager::StorageManager as SledStorageManager;

    use common::util::vec_compare::compare_unordered;

    const RO: Permissions = Permissions::ReadOnly;

    fn get_test_sm<T: StorageTrait>() -> T {
        T::new_test_sm()
    }

    #[allow(dead_code)]
    fn get_sm<T: StorageTrait>(config: &'static ServerConfig) -> T {
        T::new(config)
    }

    #[rstest]
    #[case::mem(get_test_sm::<MemStorageManager>())]
    #[case::heap(get_test_sm::<HeapStorageManager>())]
    #[case::sled(get_test_sm::<SledStorageManager>())]
    // #[case::fbtree(get_test_sm::<FbtreeStorageManager>())]
    fn sm_inserts<T: StorageTrait>(#[case] instance: T) {
        let t = TransactionId::new();
        let sizes: Vec<usize> = vec![10, 50, 75, 100, 500, 1000];
        let mut rng = get_rng();

        for (i, size) in sizes.iter().enumerate() {
            let expected: Vec<Vec<u8>> = (0..*size)
                .map(|_| {
                    let size = gen_random_int(&mut rng, 50, 100);
                    get_random_byte_vec(&mut rng, size)
                })
                .collect();
            let cid = i as ContainerId;
            instance.create_table(cid).unwrap();
            instance.insert_values(cid, expected.clone(), t);
            let result: Vec<Vec<u8>> = instance.get_iterator(cid, t, RO).map(|(a, _)| a).collect();
            assert!(compare_unordered(&expected, &result));
        }
    }

    #[rstest]
    #[case::mem(get_test_sm::<MemStorageManager>())]
    #[case::heap(get_test_sm::<HeapStorageManager>())]
    #[case::sled(get_test_sm::<SledStorageManager>())]
    // #[case::fbtree(get_test_sm::<FbtreeStorageManager>())]
    fn sm_insert_delete<T: StorageTrait>(#[case] instance: T) {
        let t = TransactionId::new();

        let mut rng = get_rng();
        let mut expected: Vec<Vec<u8>> = (0..100)
            .map(|_| {
                let size = gen_random_int(&mut rng, 50, 100);
                get_random_byte_vec(&mut rng, size)
            })
            .collect();
        let cid = 1;
        instance.create_table(cid).unwrap();
        let mut val_ids = instance.insert_values(cid, expected.clone(), t);
        for _ in 0..10 {
            let idx_to_del = gen_random_int(&mut rng, 0, expected.len() - 1);
            instance.delete_value(val_ids[idx_to_del], t).unwrap();
            let result: Vec<Vec<u8>> = instance.get_iterator(cid, t, RO).map(|(a, _)| a).collect();
            assert!(!compare_unordered(&expected, &result));
            expected.swap_remove(idx_to_del);
            val_ids.swap_remove(idx_to_del);
            assert!(compare_unordered(&expected, &result));
        }
    }

    #[rstest]
    #[case::mem(get_test_sm::<MemStorageManager>())]
    #[case::heap(get_test_sm::<HeapStorageManager>())]
    #[case::sled(get_test_sm::<SledStorageManager>())]
    // #[case::fbtree(get_test_sm::<FbtreeStorageManager>())]
    fn sm_insert_updates<T: StorageTrait>(#[case] instance: T) {
        let t = TransactionId::new();
        let mut rng = get_rng();
        let mut expected: Vec<Vec<u8>> = (0..100)
            .map(|_| {
                let size = gen_random_int(&mut rng, 50, 100);
                get_random_byte_vec(&mut rng, size)
            })
            .collect();
        let cid = 1;
        instance.create_table(cid).unwrap();
        let mut val_ids = instance.insert_values(cid, expected.clone(), t);
        for _ in 0..10 {
            let idx_to_upd = gen_random_int(&mut rng, 0, expected.len() - 1);
            let new_bytes = get_random_byte_vec(&mut rng, 15);
            let new_val_id = instance
                .update_value(new_bytes.clone(), val_ids[idx_to_upd], t)
                .unwrap();
            expected[idx_to_upd] = new_bytes;
            let result: Vec<Vec<u8>> = instance.get_iterator(cid, t, RO).map(|(a, _)| a).collect();
            assert!(compare_unordered(&expected, &result));
            val_ids[idx_to_upd] = new_val_id;
        }
    }

    // TODO dev25 - riki ?
    // #[rstest]
    // #[case::mem(get_test_sm::<MemStorageManager>())]
    // #[case::heap(get_test_sm::<HeapStorageManager>())]
    // #[case::sled(get_test_sm::<SledStorageManager>())]
    // #[case::fbtree(get_test_sm::<FbtreeStorageManager>())]
    // #[should_panic]
    // fn sm_no_container<T: StorageTrait>(#[case] instance: T) {
    //     let t = TransactionId::new();
    //     let expected: Vec<Vec<u8>> = (0..100)
    //         .map(|_| get_random_byte_vec(gen_random_int(50, 100)))
    //         .collect();
    //     instance.insert_values(1, expected, t);
    // }

    // /* TODO: Fix SMs to behave the same way
    // #[rstest]
    // #[case::mem(get_test_sm::<MemStorageManager>())]
    // #[case::heap(get_test_sm::<HeapStorageManager>())]
    // #[case::sled(get_test_sm::<SledStorageManager>())]
    // fn test_not_found<T: StorageTrait>(#[case] instance: T) {
    //     let t = TransactionId::new();
    //     let cid = 1 as ContainerId;
    //     instance.create_table(cid).unwrap();

    //     let val_id1 = ValueId::new_slot(cid, 1, 1);
    //     assert!(instance.get_value(val_id1, t, RO).is_err());
    //     assert!(instance.delete_value(val_id1, t).is_ok());
    //     assert!(instance.update_value(vec![], val_id1, t).is_err());
    // }
    // */

    // #[rstest]
    // #[case::mem(get_sm::<MemStorageManager>, "mem_sm_dir")]
    // #[case::heap(get_sm::<HeapStorageManager>, "heap_sm_dir")]
    // #[case::sled(get_sm::<SledStorageManager>, "sled_sm_dir")]
    // #[case::fbtree(get_sm::<FbtreeStorageManager>, "fbtree_sm_dir")]
    // fn sm_shutdown<T: StorageTrait, F>(#[case] get_sm: F, #[case] config: ServerConfig)
    // where
    //     F: Fn(&Path) -> T,
    // {
    //     // create path if it doesn't exist
    //     let path = Path::new(path);
    //     if !path.exists() {
    //         fs::create_dir(path).unwrap();
    //     }
    //     let t = TransactionId::new();
    //     let instance1 = get_sm(path);

    //     let expected: Vec<Vec<u8>> = (0..100)
    //         .map(|_| get_random_byte_vec(gen_random_int(50, 100)))
    //         .collect();
    //     let cid = 1;
    //     instance1.create_table(cid).unwrap();
    //     let _val_ids = instance1.insert_values(cid, expected.clone(), t);
    //     instance1.shutdown();
    //     drop(instance1);

    //     let instance2 = get_sm(path);
    //     let result: Vec<Vec<u8>> = instance2.get_iterator(cid, t, RO).map(|(a, _)| a).collect();
    //     assert!(compare_unordered(&expected, &result));
    //     instance2.reset().unwrap();

    //     fs::remove_dir_all(path).unwrap();
    // }
}
