//! Shared managed payload accounting; not a process RSS or codec-scratch limit.
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
type Payload = dyn Any + Send + Sync;
#[derive(Clone)]
pub(super) struct Pool(Arc<Mutex<Ledger>>);
struct Ledger {
    limit: usize,
    reserved: usize,
    entries: HashMap<(TypeId, usize), Entry>,
}
struct Entry {
    owner: Weak<Payload>,
    bytes: usize,
}
pub(super) struct Reservation {
    pool: Pool,
    bytes: usize,
}
impl Ledger {
    fn prune(&mut self) {
        self.entries.retain(|_, e| e.owner.strong_count() > 0);
    }
    fn retained(&self) -> usize {
        self.entries.values().map(|e| e.bytes).sum()
    }
    fn fits(&self, extra: usize) -> bool {
        self.retained()
            .checked_add(self.reserved)
            .and_then(|n| n.checked_add(extra))
            .is_some_and(|n| n <= self.limit)
    }
}
impl Pool {
    pub(super) fn new(limit: usize) -> Self {
        Self(Arc::new(Mutex::new(Ledger {
            limit,
            reserved: 0,
            entries: HashMap::new(),
        })))
    }
    pub(super) fn reserve(&self, bytes: usize) -> Result<Reservation, &'static str> {
        let mut l = self.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        if !l.fits(bytes) {
            return Err("图片流程预算不足，请关闭不需要的实例或提高预算");
        };
        l.reserved += bytes;
        Ok(Reservation {
            pool: self.clone(),
            bytes,
        })
    }
    pub(super) fn snapshot(&self) -> Result<(usize, usize, usize), &'static str> {
        let mut l = self.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        Ok((l.retained(), l.reserved, l.limit))
    }
    pub(super) fn share<T: Any + Send + Sync>(
        &self,
        value: &Arc<T>,
        bytes: usize,
    ) -> Result<(), &'static str> {
        let mut l = self.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        let key = (TypeId::of::<T>(), Arc::as_ptr(value) as usize);
        if let Some(e) = l.entries.get(&key) {
            return if e.bytes == bytes {
                Ok(())
            } else {
                Err("图片载荷计数已变化")
            };
        };
        if !l.fits(bytes) {
            return Err("图片流程预算不足，请关闭不需要的实例或提高预算");
        };
        let erased: Arc<Payload> = value.clone();
        l.entries.insert(
            key,
            Entry {
                owner: Arc::downgrade(&erased),
                bytes,
            },
        );
        Ok(())
    }
}
impl Reservation {
    pub(super) fn grow(&mut self, bytes: usize) -> Result<(), &'static str> {
        if bytes <= self.bytes {
            return Ok(());
        };
        let delta = bytes - self.bytes;
        let mut l = self.pool.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        if !l.fits(delta) {
            return Err("图片流程预算不足，请关闭不需要的实例或提高预算");
        };
        l.reserved += delta;
        self.bytes = bytes;
        Ok(())
    }

    pub(super) fn promote<T: Any + Send + Sync>(
        mut self,
        value: &Arc<T>,
        actual: usize,
    ) -> Result<(), &'static str> {
        if actual > self.bytes {
            return Err("图片分配超出预留");
        };
        let mut l = self.pool.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        let key = (TypeId::of::<T>(), Arc::as_ptr(value) as usize);
        if let Some(e) = l.entries.get(&key) {
            if e.bytes != actual {
                return Err("图片载荷计数已变化");
            }
        } else {
            let erased: Arc<Payload> = value.clone();
            l.entries.insert(
                key,
                Entry {
                    owner: Arc::downgrade(&erased),
                    bytes: actual,
                },
            );
        }
        l.reserved -= self.bytes;
        self.bytes = 0;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.bytes > 0 {
            if let Ok(mut l) = self.pool.0.lock() {
                l.reserved -= self.bytes;
            }
        }
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new(1024 * 1024 * 1024)
    }
}
impl Pool {
    pub(super) fn set_limit(&self, limit: usize) -> Result<(), &'static str> {
        let mut l = self.0.lock().map_err(|_| "图片预算暂不可用")?;
        l.prune();
        if l.retained()
            .checked_add(l.reserved)
            .is_none_or(|n| n > limit)
        {
            return Err("新预算低于当前占用，请先释放实例或等待任务完成");
        };
        l.limit = limit;
        Ok(())
    }
}
pub(super) fn pixels(image: &image::DynamicImage) -> usize {
    use image::DynamicImage::*;
    match image {
        ImageLuma8(v) => v.as_raw().capacity(),
        ImageLumaA8(v) => v.as_raw().capacity(),
        ImageRgb8(v) => v.as_raw().capacity(),
        ImageRgba8(v) => v.as_raw().capacity(),
        ImageLuma16(v) => v.as_raw().capacity() * 2,
        ImageLumaA16(v) => v.as_raw().capacity() * 2,
        ImageRgb16(v) => v.as_raw().capacity() * 2,
        ImageRgba16(v) => v.as_raw().capacity() * 2,
        ImageRgb32F(v) => v.as_raw().capacity() * 4,
        ImageRgba32F(v) => v.as_raw().capacity() * 4,
        _ => image.as_bytes().len(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_allocation_is_charged_once_actual_copy_twice_and_last_owner_reclaims() {
        let p = Pool::new(16);
        let a = Arc::new(vec![1u8; 8]);
        p.reserve(8).unwrap().promote(&a, 8).unwrap();
        let b = a.clone();
        p.share(&b, 8).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        let c = Arc::new((*a).clone());
        p.reserve(8).unwrap().promote(&c, 8).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (16, 0));
        assert!(p.reserve(1).is_err());
        drop(a);
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (16, 0));
        drop(b);
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        drop(c);
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (0, 0));
    }
    #[test]
    fn duplicate_share_does_not_require_spare_budget() {
        let p = Pool::new(4);
        let a = Arc::new(4u32);
        p.share(&a, 4).unwrap();
        p.share(&a, 4).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (4, 0));
        assert!(p.share(&a, 8).is_err());
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (4, 0));
    }
    #[test]
    fn queued_unreceived_result_holds_charge_and_disconnected_receipt_releases() {
        let p = Pool::new(8);
        let (tx, rx) = std::sync::mpsc::channel();
        let a = Arc::new(vec![1u8; 8]);
        p.reserve(8).unwrap().promote(&a, 8).unwrap();
        tx.send(a).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        assert!(p.reserve(1).is_err());
        drop(rx);
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (0, 0));
    }
    #[test]
    fn error_cancel_and_overestimated_reservation_reclaim_without_payload_replacement() {
        let p = Pool::new(12);
        let old = Arc::new(vec![1u8; 4]);
        p.share(&old, 4).unwrap();
        let r = p.reserve(8).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (4, 8));
        let new = Arc::new(vec![2u8; 4]);
        r.promote(&new, 4).unwrap();
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        assert!(
            p.reserve(2)
                .unwrap()
                .promote(&Arc::new(vec![3u8; 3]), 3)
                .is_err()
        );
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        drop(p.reserve(4).unwrap());
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (8, 0));
        assert_eq!(*old, vec![1u8; 4]);
    }
    #[test]
    fn concurrent_preallocation_reservations_share_same_limit() {
        let p = Pool::new(8);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let mut joins = vec![];
        let mut release = vec![];
        for _ in 0..2 {
            let pool = p.clone();
            let ready = ready_tx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            release.push(tx);
            joins.push(std::thread::spawn(move || {
                let r = pool.reserve(4).unwrap();
                ready.send(()).unwrap();
                rx.recv().unwrap();
                drop(r);
            }));
        }
        for _ in 0..2 {
            ready_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (0, 8));
        assert!(p.reserve(1).is_err());
        for tx in release {
            tx.send(()).unwrap();
        }
        for j in joins {
            j.join().unwrap();
        }
        assert_eq!(p.snapshot().map(|(a, b, _)| (a, b)).unwrap(), (0, 0));
    }
}
