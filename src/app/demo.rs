/// A demo "ping-pong" application
/// two tasks communicating using a Queue
use crate::{nv_loop, task_print};
use crate::syscalls::*;
use crate::user::{transaction, pbox::*};
use macros::app;

#[app]
pub fn task_ping() {
    // First create pong task, and the queue
    let (msg_q, v_boxed) = transaction::run_sys(|j, t| {
        let q = sys_queue_create::<usize>(1, t).unwrap();
        sys_create_task("pong", 1, task_pong, q, t).unwrap();
        let v = PBox::new(0, t);
        return (q,v);
    });
    
    // Loop to send messages
    nv_loop!({
        transaction::run_sys(|j, t| {
            let v_ref = v_boxed.as_mut(j);
            sys_queue_send_back(msg_q, *v_ref, 5000, t);
            task_print!("Value sent: {}", *v_ref);
            *v_ref += 1;
            sys_task_delay(50);
        });
    });
}

#[app]
fn task_pong(q: QueueHandle<usize>) {
    // Loop to receive messages
    nv_loop!({
        transaction::run_sys(|j, t| {
            let v = sys_queue_receive(q, 5000, t).unwrap();
            #[cfg(board="msp430fr5994")]
            crate::board::msp430fr5994::peripherals::led_toggle();
            task_print!("Value received: {}", v);
        });
    });
}

