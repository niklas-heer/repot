//! Bounded parallelism over independent items, with a shared queue so one slow
//! item never holds back the items waiting behind it.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Result, ui};

/// Run `work` on every item with at most `jobs` workers and return the results
/// in input order. A panicking worker fails the whole run rather than silently
/// dropping results.
pub fn parallel<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    progress: Option<&ui::Progress>,
    work: impl Fn(&T) -> R + Sync,
) -> Result<Vec<R>> {
    let next = AtomicUsize::new(0);
    let workers = jobs.clamp(1, 32).min(items.len().max(1));
    let mut results: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            break;
                        };
                        done.push((index, work(item)));
                        if let Some(progress) = progress {
                            progress.advance();
                        }
                    }
                    done
                })
            })
            .collect();
        let mut results = Vec::with_capacity(items.len());
        for handle in handles {
            results.extend(handle.join().map_err(|_| "worker failed".to_owned())?);
        }
        Ok::<_, String>(results)
    })?;
    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn results_keep_input_order_and_a_slow_item_does_not_block_the_queue() {
        let items: Vec<u64> = (0..40).collect();
        let finished = AtomicUsize::new(0);
        let results = parallel(&items, 4, None, |item| {
            if *item == 0 {
                // With fixed chunks, the items queued behind this one could
                // never finish first; with a shared queue the others drain.
                let deadline = Instant::now() + Duration::from_secs(10);
                while finished.load(Ordering::SeqCst) < items.len() - 1 {
                    assert!(
                        Instant::now() < deadline,
                        "other items waited behind item 0"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
            } else {
                finished.fetch_add(1, Ordering::SeqCst);
            }
            item * 2
        })
        .expect("workers succeed");
        assert_eq!(
            results,
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(
            parallel::<u8, ()>(&[], 4, None, |_| ())
                .expect("empty")
                .is_empty()
        );
    }
}
