//! Maps a job's wire name to the handler that runs it.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::jobs::JobType;
use crate::jobs::JobHandler;

pub struct Registry {
    handlers: HashMap<JobType, Arc<dyn JobHandler>>,
}

impl Registry {
    /// Refuses two handlers for one type: which one ran would depend on
    /// registration order.
    pub fn new(handlers: Vec<Arc<dyn JobHandler>>) -> anyhow::Result<Self> {
        let mut map = HashMap::new();
        for handler in handlers {
            let job_type = handler.job_type();
            if map.insert(job_type, handler).is_some() {
                anyhow::bail!("two job handlers registered for {}", job_type.as_str());
            }
        }
        Ok(Self { handlers: map })
    }

    pub fn get(&self, job_type: &str) -> Option<Arc<dyn JobHandler>> {
        let job_type = JobType::parse(job_type)?;
        self.handlers.get(&job_type).cloned()
    }

    /// Registered wire names in `JobType::ALL` order (stable across runs).
    pub fn type_names(&self) -> Vec<&'static str> {
        JobType::ALL
            .into_iter()
            .filter(|t| self.handlers.contains_key(t))
            .map(JobType::as_str)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::testing::{Behaviour, Scripted, TEST_TYPE};

    /// Given two handlers for one type, When registering, Then it is refused.
    #[test]
    fn a_duplicate_job_type_is_refused() {
        let h: Arc<dyn JobHandler> = Arc::new(Scripted::new(Behaviour::Succeed));
        assert!(Registry::new(vec![h.clone(), h]).is_err());
    }

    /// Given a registered handler, When looked up by wire name, Then it is
    /// found and other names are not.
    #[test]
    fn handlers_are_found_by_wire_name() {
        let r = Registry::new(vec![Arc::new(Scripted::new(Behaviour::Succeed))]).unwrap();
        assert!(r.get(TEST_TYPE.as_str()).is_some());
        assert!(r.get("reminder.fire").is_none());
        assert_eq!(r.type_names(), vec![TEST_TYPE.as_str()]);
        assert!(!r.is_empty());
    }
}
