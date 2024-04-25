use macros::app;

use crate::{
    bench_dbg_print, bench_println,
    benchmarks::{
        benchmark_end, benchmark_start, get_benchmark_done, is_benchnmark_done,
        print_all_task_stats, print_wall_clock_time, set_benchmark_done, wall_clock_begin,
        wall_clock_end, Hash,
    },
    declare_pm_loop_cnt,
    marker::{PSafe, TxInSafe, TxOutSafe},
    nv_for_loop,
    pmem::JournalHandle,
    syscalls::{self, sys_create_task, sys_task_delay, SyscallToken},
    task,
    user::{
        parc::PArc,
        pbox::PBox,
        pmutex::{PMutex, PMutexGuard},
        pvec::PArray,
        transaction,
    },
};

const DEFAULT_HASH_TABLE_SIZE: usize = 16;

struct PHashNode<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe,
{
    next: Option<PBox<PHashNode<K, V>>>,
    key: K,
    value: V,
}

impl<K, V> PHashNode<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe,
{
    pub fn new(k: K, v: V, t: SyscallToken) -> PBox<Self> {
        let node = Self {
            next: None,
            key: k,
            value: v,
        };
        PBox::new(node, t)
    }
}

struct PHashHead<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe,
{
    next: Option<PBox<PHashNode<K, V>>>,
}

impl<K, V> Default for PHashHead<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe,
{
    fn default() -> Self {
        PHashHead { next: None }
    }
}

pub struct PHashTable<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe + Copy,
{
    table: PArray<PHashHead<K, V>>,
}

impl<K, V> PHashTable<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe + Copy,
{
    pub fn new(t: SyscallToken) -> Self {
        ();
        let table = PArray::new(DEFAULT_HASH_TABLE_SIZE, t).unwrap();
        let r = PHashTable { table };
        r
    }

    pub fn insert(&self, key: K, value: V, j: JournalHandle, t: SyscallToken) {
        let bucket_id = key.hash() % self.table.size();
        let bucket_head = self.table.index(bucket_id);
        let (next_node, mut bucket_head) = bucket_head.read_wr(|v| v.next.take(), j);
        let mut new_node = PHashNode::new(key, value, t);
        // set next of the new node
        new_node.as_mut(j).next = next_node;
        // insert at the head
        bucket_head.next = Some(new_node);
    }

    pub fn lookup(&self, key: K, j: JournalHandle) -> Result<V, ()> {
        let bucket_id = key.hash() % self.table.size();
        let bucket_head = self.table.index(bucket_id).into_readable();
        let mut cur_node = &bucket_head.as_ref(j).next;

        // walk the list
        while let Some(cur) = cur_node {
            let k = &cur.as_ref(j).key;
            if k == &key {
                return Ok(cur.as_ref(j).value);
            }
            cur_node = &cur.as_ref(j).next;
        }
        Err(())
    }

    pub fn delete(&self, key: K, j: JournalHandle, t: SyscallToken) -> Result<V, ()> {
        let bucket_id = key.hash() % self.table.size();
        let bucket_head = self.table.index(bucket_id).into_readable();
        let mut cur = &bucket_head.as_ref(j).next;

        // if just sits at the haed
        if let Some(node) = cur {
            let node = node.as_ref(j);
            if &node.key == &key {
                let (v, _) = bucket_head.read_wr(
                    |h| {
                        let to_delete = h.next.take().unwrap();
                        let to_delete = PBox::into_inner(to_delete, t);
                        h.next = to_delete.next;
                        to_delete.value
                    },
                    j,
                );
                return Ok(v);
            }
        }

        while let Some(node) = cur {
            let n = node.as_mut(j);
            if let Some(next) = &n.next {
                let next_ref = next.as_ref(j);
                if &next_ref.key == &key {
                    // delete
                    let to_delete = n.next.take().unwrap();
                    let to_delete = PBox::into_inner(to_delete, t);
                    n.next = to_delete.next;
                    return Ok(to_delete.value);
                } else {
                    cur = &node.as_ref(j).next;
                }
            } else {
                break;
            }
        }

        Err(())
    }
}

pub struct SharedKVStore<K, V>
where
    K: Hash + PSafe + PartialEq,
    V: PSafe + Copy,
{
    mutexed_ht: PArc<PMutex<PHashTable<K, V>>>,
}

impl<K, V> SharedKVStore<K, V>
where
    K: Hash + PSafe + PartialEq + TxInSafe,
    V: PSafe + TxInSafe + TxOutSafe + Copy,
{
    pub fn new(t: SyscallToken) -> Self {
        let phash_table = PHashTable::new(t);
        Self {
            mutexed_ht: PArc::new(PMutex::new(phash_table, t), t),
        }
    }

    pub fn lock_kv<F>(&self, f: F)
    where
        F: FnOnce(&PMutexGuard<PHashTable<K, V>>),
    {
        let ht = self.mutexed_ht.lock().unwrap();
        f(&ht);
    }

    pub fn clone(&self, j: JournalHandle) -> Self {
        Self {
            mutexed_ht: self.mutexed_ht.clone(j),
        }
    }
}

#[cfg(board = "qemu")]
const INSERT_NUM: usize = 20;
#[cfg(board = "apollo4bp")]
const INSERT_NUM: usize = 50;
#[cfg(board = "msp430fr5994")]
const INSERT_NUM: usize = 50;
const BASE_2: usize = INSERT_NUM;
const QUERY_NUM: usize = INSERT_NUM;

declare_pm_loop_cnt!(LOOP_CNT_1, 0);
declare_pm_loop_cnt!(LOOP_CNT_2, 0);
declare_pm_loop_cnt!(LOOP_CNT_3, 0);
declare_pm_loop_cnt!(LOOP_CNT_4, 0);

static mut STAT: usize = 0;

fn update_stat(cnt: usize) {
    unsafe {
        STAT += cnt;
    }
}

#[app]
fn task_kv_worker_1() {
    wall_clock_begin();
    benchmark_start();
    let shared_kv = transaction::run_sys(|j, t| {
        let shared_kv = SharedKVStore::<usize, usize>::new(t);
        let kv = shared_kv.clone(j);
        sys_create_task("kv worker2", 1, task_kv_worker_2, kv, t);
        shared_kv
    });

    shared_kv.lock_kv(|ht| {
        nv_for_loop!(LOOP_CNT_1, i, 0 => INSERT_NUM/5, {
            transaction::run_sys(|j, t| {
                for x in 0..5 {
                    let k = i * 5 + x;
                    bench_dbg_print!("Inserting key: {}", k);
                    ht.insert(k, k+42, j, t);
                }
            });
        });
    });
    #[cfg(not(feature = "bench_kv_smaller_tx_sz"))]
    shared_kv.lock_kv(|ht| {
        transaction::run(|j| {
            let mut key_found = 0;
            for i in 0..QUERY_NUM {
                let r = ht.lookup(i, j);
                match r {
                    Ok(v) => {
                        key_found += 1;
                        bench_dbg_print!("key: {}, value: {}", i, v);
                    }
                    Err(_) => {
                        bench_dbg_print!("key: {} doesn't exist", i);
                    }
                };
            }
            update_stat(key_found);
            // bench_println!("number of keys found: {}", key_found);
        });
    });

    #[cfg(feature = "bench_kv_smaller_tx_sz")]
    shared_kv.lock_kv(|ht| {
        nv_for_loop!(LOOP_CNT_2, i, 0 => 2,  {
            transaction::run(|j| {
                let mut key_found = 0;
                let start = BASE_2 + (QUERY_NUM/2)*i;
                let end = start + (QUERY_NUM/2);
                for i in start..end  {
                    let r = ht.lookup(i, j);
                    match r {
                        Ok(v) => { key_found += 1; },
                        Err(_) => {}
                    };
                }
                update_stat(key_found);
            });
        });
    });

    shared_kv.lock_kv(|ht| {
        nv_for_loop!(LOOP_CNT_3, i, 0 => INSERT_NUM/5 , {
            transaction::run_sys(|j, t| {
                for x in (0..5).step_by(3) {
                    let key = i * 5 + x;
                    ht.delete(key, j, t);
                }
            });
        });
    });
    #[cfg(not(feature = "bench_kv_smaller_tx_sz"))]
    shared_kv.lock_kv(|ht| {
        transaction::run(|j| {
            let mut key_found = 0;
            for i in 0..QUERY_NUM {
                let r = ht.lookup(i, j);
                match r {
                    Ok(v) => {
                        key_found += 1;
                        bench_dbg_print!("key: {}, value: {}", i, v);
                    }
                    Err(_) => {
                        bench_dbg_print!("key: {} doesn't exist", i);
                    }
                };
            }
            update_stat(key_found);
            // bench_println!("number of keys found: {}", key_found);
        });
    });

    #[cfg(feature = "bench_kv_smaller_tx_sz")]
    shared_kv.lock_kv(|ht| {
        nv_for_loop!(LOOP_CNT_4, i, 0 => 2,  {
            transaction::run(|j| {
                let mut key_found = 0;
                let start = BASE_2 + (QUERY_NUM/2)*i;
                let end = start + (QUERY_NUM/2);
                for i in start..end  {
                    let r = ht.lookup(i, j);
                    match r {
                        Ok(v) => { key_found += 1; },
                        Err(_) => {}
                    };
                }
                update_stat(key_found);
            });
        });
    });

    #[cfg(not(feature = "power_failure"))]
    {
        wall_clock_end();
        sys_task_delay(5);
        print_wall_clock_time();
        print_all_task_stats();
    }
    #[cfg(feature = "power_failure")]
    {
        benchmark_end();
        set_benchmark_done();

        loop {
            // bench_println!("benchmark done cnt: {}", get_benchmark_done());
            if is_benchnmark_done() {
                wall_clock_end();
                print_all_task_stats();
                print_wall_clock_time();
                break;
            }
        }
    }
}

declare_pm_loop_cnt!(T2_LOOP_CNT_1, 0);
declare_pm_loop_cnt!(T2_LOOP_CNT_2, 0);
declare_pm_loop_cnt!(T2_LOOP_CNT_3, 0);
declare_pm_loop_cnt!(T2_LOOP_CNT_4, 0);

#[app]
fn task_kv_worker_2(shared_kv: SharedKVStore<usize, usize>) {
    /////////////////// Task 2 KV Operations //////////////////////////////
    let wall_clk_start = benchmark_start();
    shared_kv.lock_kv(|ht| {
        nv_for_loop!(T2_LOOP_CNT_1, i, 0 => INSERT_NUM/5 , {
            transaction::run_sys(|j, t| {
                for x in 0..5 {
                    let k = i * 5 + x + BASE_2;
                    ht.insert(k, k+42, j, t);
                }
            });
        });
    });
    #[cfg(not(feature = "bench_kv_smaller_tx_sz"))]
    shared_kv.lock_kv(|ht| {
        transaction::run(|j| {
            let mut key_found = 0;
            for i in BASE_2..(BASE_2 + QUERY_NUM) {
                let r = ht.lookup(i, j);
                match r {
                    Ok(v) => {
                        key_found += 1;
                        bench_dbg_print!("key: {}, value: {}", i, v);
                    }
                    Err(_) => {
                        bench_dbg_print!("key: {} doesn't exist", i);
                    }
                };
            }
            update_stat(key_found);
            // bench_println!("number of keys found: {}", key_found);
        });
    });
    #[cfg(feature = "bench_kv_smaller_tx_sz")]
    shared_kv.lock_kv(|ht| {
        nv_for_loop!(T2_LOOP_CNT_2, i, 0 => 2,  {
            transaction::run(|j| {
                let mut key_found = 0;
                let start = BASE_2 + (QUERY_NUM/2)*i;
                let end = start + (QUERY_NUM/2);
                for i in start..end  {
                    let r = ht.lookup(i, j);
                    match r {
                        Ok(v) => { key_found += 1; },
                        Err(_) => {}
                    };
                }
                update_stat(key_found);
            });
        });
    });

    shared_kv.lock_kv(|ht| {
        nv_for_loop!(T2_LOOP_CNT_3, i, 0 => INSERT_NUM,  {
            transaction::run_sys(|j, t| {
                for x in (0..5).step_by(3) {
                    let key = i * 5 + x + BASE_2;
                    ht.delete(key, j, t);
                }
            });
        });
    });
    #[cfg(not(feature = "bench_kv_smaller_tx_sz"))]
    shared_kv.lock_kv(|ht| {
        transaction::run(|j| {
            let mut key_found = 0;
            for i in BASE_2..(BASE_2 + QUERY_NUM) {
                let r = ht.lookup(i, j);
                match r {
                    Ok(v) => {
                        key_found += 1;
                        bench_dbg_print!("key: {}, value: {}", i, v);
                    }
                    Err(_) => {
                        bench_dbg_print!("key: {} doesn't exist", i);
                    }
                };
            }
            update_stat(key_found);
            // bench_println!("number of keys found: {}", key_found);
        });
    });

    #[cfg(feature = "bench_kv_smaller_tx_sz")]
    shared_kv.lock_kv(|ht| {
        nv_for_loop!(T2_LOOP_CNT_4, i, 0 => 2,  {
            transaction::run(|j| {
                let mut key_found = 0;
                let start = BASE_2 + (QUERY_NUM/2)*i;
                let end = start + (QUERY_NUM/2);
                for i in start..end  {
                    let r = ht.lookup(i, j);
                    match r {
                        Ok(v) => { key_found += 1; },
                        Err(_) => {}
                    };
                }
                update_stat(key_found);
            });
        });
    });

    let wall_clk_end = benchmark_end();
    #[cfg(feature = "power_failure")]
    {
        set_benchmark_done();
    }
    bench_println!("Wall clock cycles: {}", wall_clk_end - wall_clk_start);
}

pub fn register() {
    task::register_app_no_param("kv", 1, task_kv_worker_1);
}
