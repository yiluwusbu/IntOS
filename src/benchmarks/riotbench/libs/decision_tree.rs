use core::mem::transmute_copy;

use macros::app;

use crate::{
    bench_println,
    benchmarks::riotbench::{ValueType, TRAIN_DATASET_SZ},
    declare_const_pm_var, declare_pm_loop_cnt, nv_for_loop, task_print,
    user::{
        pbox::{PBox, PRef, Ptr},
        pqueue::PQueue,
        pvec::PVec,
        transaction,
    },
    util::benchmark_clock,
};

use super::data_layout::SensorData;

const MAX_CLASS_NUM: usize = 4;
const MAX_TREE_DEPTH: usize = 10;
pub trait DecisionTreeAttribute {
    fn class(&self) -> isize {
        -1
    }
}
struct DecisionTreeNode {
    use_field: usize,
    criteria: fn(ValueType) -> bool,
    result_class: isize, // -1 means it is not a leaf
    children: [usize; 2],
}

pub struct DecisionTree<const N: usize> {
    nodes: [DecisionTreeNode; N],
}

const IS_LEAF: usize = 1000;

impl<const N: usize> DecisionTree<N> {
    pub fn classify(&self, data: &SensorData) -> isize {
        let mut cur_node = 0;
        let mut res = -1;
        for i in 0..MAX_TREE_DEPTH {
            let node = &self.nodes[cur_node];
            if node.result_class != -1 {
                res = node.result_class;
                break;
            }
            let n = node.use_field;
            if n >= N {
                return -1;
            }
            let field_v = data.values[n];
            let f = node.criteria;
            if f(field_v) {
                cur_node = node.children[1];
            } else {
                cur_node = node.children[0];
            }
            if cur_node == IS_LEAF {
                break;
            }
        }
        return res;
    }
}

fn classify_0(v: ValueType) -> bool {
    v > 10 && v < 50
}

fn classify_1(v: ValueType) -> bool {
    v < 10
}

fn classify_2(v: ValueType) -> bool {
    v > 50 && v < 70
}

fn classify_3(v: ValueType) -> bool {
    v > 80
}

#[link_name = ".pmem"]
static TREE_1: DecisionTree<15> = DecisionTree {
    nodes: [
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: -1,
            children: [1, 2],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: -1,
            children: [3, 4],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: -1,
            children: [5, 6],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [7, 8],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [9, 10],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: -1,
            children: [11, 12],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: -1,
            children: [13, 14],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
    ],
};

#[link_name = ".pmem"]
static TREE_2: DecisionTree<15> = DecisionTree {
    nodes: [
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [1, 2],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: -1,
            children: [3, 4],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: -1,
            children: [5, 6],
        },
        DecisionTreeNode {
            use_field: 3,
            criteria: classify_3,
            result_class: -1,
            children: [7, 8],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [9, 10],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [11, 12],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: -1,
            children: [13, 14],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 2,
            criteria: classify_2,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 0,
            criteria: classify_0,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 1,
            criteria: classify_1,
            result_class: 0,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 3,
            criteria: classify_3,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
        DecisionTreeNode {
            use_field: 3,
            criteria: classify_3,
            result_class: 1,
            children: [IS_LEAF, IS_LEAF],
        },
    ],
};

pub fn get_tree_1() -> &'static DecisionTree<15> {
    &TREE_1
}

pub fn get_tree_2() -> &'static DecisionTree<15> {
    &TREE_1
}

fn ln(x: ValueType) -> ValueType {
    // taylor expansion
    let first_item = x - 1;
    let second_item = (x - 1) * (x - 1) / 2;
    let third_item = (x - 1) * (x - 1) * (x - 1) / 3;
    let fourth_item = (x - 1) * (x - 1) * (x - 1) * (x - 1) / 4;
    return first_item - second_item + third_item - fourth_item;
}

fn entropy(p: &[ValueType]) -> ValueType {
    let mut sum = 0;
    for i in 0..p.len() {
        sum += -ln(p[i]) * p[i];
    }
    return sum;
}

fn group_entropy(v: &SmallVecU8, labels: &[ValueType]) -> ValueType {
    let mut c0: ValueType = 0;
    let mut c1: ValueType = 0;
    for i in 0..v.len as usize {
        let id = v.vec[i] as usize;
        if labels[id] == 0 {
            c0 += 1;
        } else {
            c1 += 1;
        }
    }
    return entropy(&[c0, c1]);
}

const SMALL_VEC_SZ: usize = 16;

#[derive(Clone, Copy)]
pub struct SmallVecU8 {
    vec: [u8; SMALL_VEC_SZ],
    len: u8,
}

impl SmallVecU8 {
    pub fn new() -> Self {
        Self {
            vec: [0; SMALL_VEC_SZ],
            len: 0,
        }
    }
    pub fn push(&mut self, item: u8) {
        self.vec[self.len as usize] = item;
        self.len += 1;
    }
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len as usize
    }
    #[inline(always)]
    pub fn at(&self, i: usize) -> u8 {
        self.vec[i]
    }
}

fn split_grp_with_feature(
    grp: &SmallVecU8,
    f: usize,
    samples: &[[ValueType; 4]],
    g1: &mut SmallVecU8,
    g2: &mut SmallVecU8,
) {
    for j in 0..grp.len() {
        let id = grp.at(j);
        if samples[id as usize][f] == 1 {
            g1.push(id);
        } else {
            g2.push(id);
        }
    }
}

fn info_gain_on_feature(
    samples: &[[ValueType; 4]],
    labels: &[ValueType],
    ft: usize,
    entropy: ValueType,
    grp: &SmallVecU8,
) -> ValueType {
    let mut g1 = SmallVecU8::new();
    let mut g2 = SmallVecU8::new();
    let mut weighted_entropy = 0;

    split_grp_with_feature(&grp, ft, samples, &mut g1, &mut g2);

    weighted_entropy += -1 * g1.len() as ValueType * group_entropy(&g1, labels);
    weighted_entropy += -1 * g2.len() as ValueType * group_entropy(&g2, labels);
    weighted_entropy /= grp.len() as ValueType;
    let gain = entropy - weighted_entropy;
    return gain;
}

declare_pm_loop_cnt!(TRAIN_NODE_ITER, 0);

pub struct DTTrainMeta {
    max_gain: ValueType,
    use_feature: isize,
}

impl DTTrainMeta {
    pub fn new() -> Self {
        DTTrainMeta {
            max_gain: ValueType::MIN,
            use_feature: 0,
        }
    }
}

fn train_node(
    samples: &[[ValueType; 4]],
    labels: &[ValueType],
    grp: &SmallVecU8,
    train_meta: &PBox<DTTrainMeta>,
) -> isize {
    if grp.len <= 4 {
        return -1;
    }

    let entropy = transaction::run(|j| group_entropy(grp, labels));

    nv_for_loop!(TRAIN_NODE_ITER, i, 0=>4, {
        transaction::run(|j| {
            // let tm = train_meta.as_mut(j);
            // if i == 0 {
            //     tm.max_gain = ValueType::MIN;
            //     tm.use_feature = 0;
            // }

            let mut g1 = SmallVecU8::new();
            let mut g2 = SmallVecU8::new();
            let mut weighted_entropy = 0;

            split_grp_with_feature(&grp, i,  samples, &mut g1, &mut g2);

            weighted_entropy += -1 * g1.len() as ValueType * group_entropy(&g1, labels);
            weighted_entropy += -1 * g2.len() as ValueType * group_entropy(&g2, labels);
            weighted_entropy /= grp.len() as ValueType;
            let gain = entropy - weighted_entropy;

            if gain == 0 {
                let tm = train_meta.as_mut(j);
                tm.use_feature = i as isize;
                tm.max_gain = gain;
            } else if gain > train_meta.as_ref(j).max_gain {
                let tm = train_meta.as_mut(j);
                tm.use_feature = i as isize;
                tm.max_gain = gain;
            }
        });
    });

    let ft = transaction::run(|j| train_meta.as_ref(j).use_feature);

    return ft;
}

// fn train_node(samples: &[[ValueType; 4]], labels: &[ValueType], grp: &SmallVecU8) -> isize {
//     if grp.len <= 4 {
//         return -1;
//     }
//     let mut max_gain: ValueType = ValueType::MIN;
//     let mut use_feature = 0;
//     let entropy = group_entropy(grp, labels);

//     for i in 0..2 {
//         // classify based on feature i
//         let mut g1 = SmallVecU8::new();
//         let mut g2 = SmallVecU8::new();
//         let mut weighted_entropy = 0;

//         split_grp_with_feature(&grp, i,  samples, &mut g1, &mut g2);

//         weighted_entropy += -1 * g1.len() as ValueType * group_entropy(&g1, labels);
//         weighted_entropy += -1 * g2.len() as ValueType * group_entropy(&g2, labels);
//         weighted_entropy /= grp.len() as ValueType;
//         let gain = entropy - weighted_entropy;
//         if gain > max_gain {
//             use_feature = i as isize;
//             max_gain = gain;
//         }
//     }
//     return use_feature;
// }

const DT_PARAM_SZ: usize = 10;

#[derive(Clone, Copy)]
pub struct CompressedDTParam {
    nid: u8,
    feature_id: i8,
}

pub struct DTParams {
    params: [CompressedDTParam; DT_PARAM_SZ],
}

impl DTParams {
    pub fn new() -> Self {
        Self {
            params: [CompressedDTParam {
                nid: 0,
                feature_id: 0,
            }; DT_PARAM_SZ],
        }
    }

    pub fn at(&self, i: usize) -> &CompressedDTParam {
        &self.params[i]
    }

    pub fn at_mut(&mut self, i: usize) -> &mut CompressedDTParam {
        &mut self.params[i]
    }
}

const MAX_TREE_DEPTH_TRAIN: usize = 6;

declare_pm_loop_cnt!(ITER_CNT, 0);
declare_pm_loop_cnt!(Q_ITER_CNT, 0);

fn debug_print_grp(grp: &SmallVecU8) {
    crate::board_hprint!("[");
    for i in 0..grp.len() {
        crate::board_hprint!("{}, ", grp.at(i));
    }
    crate::board_hprintln!("]");
}

pub fn train_decision_tree(
    samples: &[[ValueType; 4]; TRAIN_DATASET_SZ],
    labels: &[ValueType; TRAIN_DATASET_SZ],
    train_meta: &PBox<DTTrainMeta>,
    pqueue: &PQueue<(SmallVecU8, usize)>,
    res: &PVec<CompressedDTParam>,
) {
    let mut g0 = SmallVecU8::new();
    for i in 0..TRAIN_DATASET_SZ {
        g0.push(i as u8);
    }
    transaction::run(|j| {
        pqueue.push_back((g0, 1), j);
    });

    nv_for_loop!(ITER_CNT, i, 0 => MAX_TREE_DEPTH_TRAIN,  {
        let qlen = transaction::run(|j| {
            pqueue.len(j)
        });

        nv_for_loop!(Q_ITER_CNT, k, 0=>qlen, {
            let grp_nid = transaction::run(|j| {
                pqueue.peek_front(j).unwrap()
            });
            let grp = &grp_nid.0;
            let ft = train_node(samples, labels, grp, train_meta);

            // task_print!("Nodes Cnt: {}", res.len(j));
            transaction::run(|j| {
                let (grp,nid) = pqueue.pop_front(j).unwrap();
                if !res.is_full(j) {
                    res.push(CompressedDTParam {nid: nid as u8, feature_id: ft as i8 }, j);
                }
                if ft != -1 {
                    let mut g1 = SmallVecU8::new();
                    let mut g2 = SmallVecU8::new();
                    split_grp_with_feature(&grp, ft as usize, samples, &mut g1, &mut g2);
                    if g1.len() > 0  {
                        // crate::board_hprintln!("Node id: {}", nid * 2);
                        // debug_print_grp(&g1);
                        pqueue.push_back((g1, nid * 2), j);
                    }
                    if g2.len() > 0  {
                        // crate::board_hprintln!("Node id: {}", nid * 2+1);
                        // debug_print_grp(&g2);
                        pqueue.push_back((g2, nid*2+1), j);
                    }
                }
            });
        });

    });
}

// pub fn train_decision_tree(samples: &[[ValueType; 4]; TRAIN_DATASET_SZ], labels: &[ValueType; TRAIN_DATASET_SZ], pqueue: &PQueue<(SmallVecU8,usize)>, res: &PVec<CompressedDTParam>) {
//     let mut g0 = SmallVecU8::new();
//     for i in 0..TRAIN_DATASET_SZ {
//         g0.push(i as u8);
//     }
//     transaction::run(|j| {
//         pqueue.push_back((g0,1), j);
//     });

//     nv_for_loop!(ITER_CNT, i, 0 => MAX_TREE_DEPTH_TRAIN,  {
//         let qlen = transaction::run(|j| {
//             pqueue.len(j)
//         });

//         nv_for_loop!(Q_ITER_CNT, k, 0=>qlen, {
//             transaction::run(|j| {
//                 let (grp,nid) = pqueue.pop_front(j).unwrap();
//                 //debug_print_grp(&grp);
//                 let ft = train_node(samples, labels, &grp);
//                 // task_print!("Nodes Cnt: {}", res.len(j));
//                 res.push(CompressedDTParam {nid: nid as u8, feature_id: ft as i8 }, j);
//                 if ft != -1 {
//                     let mut g1 = SmallVecU8::new();
//                     let mut g2 = SmallVecU8::new();
//                     split_grp_with_feature(&grp, ft as usize, samples, &mut g1, &mut g2);
//                     if g1.len() > 0  {
//                         // crate::board_hprintln!("Node id: {}", nid * 2);
//                         // debug_print_grp(&g1);
//                         pqueue.push_back((g1, nid * 2), j);
//                     }
//                     if g2.len() > 0  {
//                         // crate::board_hprintln!("Node id: {}", nid * 2+1);
//                         // debug_print_grp(&g2);
//                         pqueue.push_back((g2, nid*2+1), j);
//                     }
//                 }
//             });
//         });

//     });
// }

// #[link_section = ".pmem"]
// static SAMPELS: [[ValueType; 4]; TRAIN_DATASET_SZ] =
// [[1, 1, 1, 1],
// [2, 2, 2, 2],
// [2, 1, 2, 2],
// [2, 2, 1, 1],
// [2, 2, 1, 1],
// [1, 2, 2, 1],
// [2, 2, 2, 1],
// [2, 1, 2, 1],
// [1, 1, 2, 1],
// [2, 2, 1, 1],
// [1, 2, 2, 1],
// [2, 1, 2, 1],
// [2, 1, 1, 1],
// [2, 1, 2, 1],
// [1, 2, 2, 2],
// [1, 2, 1, 1],
// [1, 2, 2, 1],
// [2, 2, 1, 2],
// [1, 1, 2, 2],
// [2, 2, 2, 2],
// [1, 2, 1, 2],
// [1, 1, 2, 2],
// [2, 1, 2, 1],
// [2, 1, 1, 2],
// [2, 2, 2, 1],
// [1, 1, 2, 1],
// [1, 1, 2, 1],
// [2, 2, 2, 2],
// [1, 2, 2, 1],
// [1, 2, 1, 2],
// [1, 1, 2, 1],
// [1, 2, 2, 2],];

// #[link_section = ".pmem"]
// static LABELS: [ValueType; TRAIN_DATASET_SZ] = [1, 0, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0];

// #[app]
// fn task_test_decision_tree_training() {
//     let (pq, mut dt_params) = transaction::run_sys(|j,t| {
//         let pq = PQueue::new(16, t);
//         let dt_params = PVec::new(16, t);
//         (pq, dt_params)
//     });

//     train_decision_tree(&SAMPELS, &LABELS, &pq, &dt_params);
// }

// pub fn register() {
//     crate::task::register_app_no_param("test DT train", 1, task_test_decision_tree_training);
// }
