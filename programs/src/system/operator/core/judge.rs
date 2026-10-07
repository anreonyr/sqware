//! 身份查询束与树——判一格要问的全部事实。

use env::TaskId;
use system_api::identity::Selector;
use system_api::operator::{EntryId, Permit, Ruling};

/// 所有身份事实必须来自同一份可信、来源绑定的查询束。
pub trait Facts {
    fn bound(&self, task: TaskId) -> Result<bool, ()>;
    fn matches(&self, task: TaskId, selector: Selector) -> Result<bool, ()>;
    fn same(&self, a: TaskId, b: TaskId) -> Result<bool, ()>;
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()>;
}

/// Public 不依赖身份服务；其余只认确定的回答，查询失败不放行。
pub fn judge(f: &impl Facts, who: TaskId, permit: Permit) -> Ruling {
    match permit {
        Permit::Public => Ruling::Allow,
        Permit::Bound => ruling(f.bound(who)),
        Permit::Identity(selector) => ruling(f.matches(who, selector)),
        Permit::Opener(at) => match f.opens(at) {
            Ok(Some(that)) => ruling(f.same(who, that)),
            Ok(None) | Err(()) => Ruling::Unjudged,
        },
    }
}

fn ruling(answer: Result<bool, ()>) -> Ruling {
    match answer {
        Ok(true) => Ruling::Allow,
        Ok(false) => Ruling::Deny,
        Err(()) => Ruling::Unjudged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Offline;
    impl Facts for Offline {
        fn bound(&self, _: TaskId) -> Result<bool, ()> {
            Err(())
        }
        fn matches(&self, _: TaskId, _: Selector) -> Result<bool, ()> {
            Err(())
        }
        fn same(&self, _: TaskId, _: TaskId) -> Result<bool, ()> {
            Err(())
        }
        fn opens(&self, _: EntryId) -> Result<Option<TaskId>, ()> {
            Err(())
        }
    }

    #[test]
    fn offline_is_public_only() {
        let who = TaskId::new(7);
        assert_eq!(judge(&Offline, who, Permit::Public), Ruling::Allow);
        assert_eq!(judge(&Offline, who, Permit::Bound), Ruling::Unjudged);
        assert_eq!(
            judge(&Offline, who, Permit::Opener(EntryId::new(0))),
            Ruling::Unjudged
        );
    }

    struct Answers {
        bound: Result<bool, ()>,
        matches: Result<bool, ()>,
        same: Result<bool, ()>,
        opener: Option<TaskId>,
    }

    impl Facts for Answers {
        fn bound(&self, _: TaskId) -> Result<bool, ()> {
            self.bound
        }
        fn matches(&self, _: TaskId, _: Selector) -> Result<bool, ()> {
            self.matches
        }
        fn same(&self, _: TaskId, _: TaskId) -> Result<bool, ()> {
            self.same
        }
        fn opens(&self, _: EntryId) -> Result<Option<TaskId>, ()> {
            Ok(self.opener)
        }
    }

    #[test]
    fn bound_distinguishes_no_binding_from_unavailable() {
        let facts = Answers {
            bound: Ok(false),
            matches: Err(()),
            same: Err(()),
            opener: None,
        };
        assert_eq!(judge(&facts, TaskId::new(1), Permit::Bound), Ruling::Deny);
    }

    #[test]
    fn identity_uses_one_atomic_match_not_resolve() {
        use system_api::identity::PrincipalId;
        let selector = Selector::Exact(PrincipalId::new(TaskId::new(9), 0));
        let mut facts = Answers {
            bound: Err(()),
            matches: Ok(true),
            same: Err(()),
            opener: None,
        };
        assert_eq!(
            judge(&facts, TaskId::new(1), Permit::Identity(selector)),
            Ruling::Allow
        );
        facts.matches = Ok(false);
        assert_eq!(
            judge(&facts, TaskId::new(1), Permit::Identity(selector)),
            Ruling::Deny
        );
        facts.matches = Err(());
        assert_eq!(
            judge(&facts, TaskId::new(1), Permit::Identity(selector)),
            Ruling::Unjudged
        );
    }

    #[test]
    fn opener_uses_same_not_two_resolves() {
        let mut facts = Answers {
            bound: Err(()),
            matches: Err(()),
            same: Ok(true),
            opener: Some(TaskId::new(2)),
        };
        let permit = Permit::Opener(EntryId::new(0));
        assert_eq!(judge(&facts, TaskId::new(1), permit), Ruling::Allow);
        facts.same = Ok(false);
        assert_eq!(judge(&facts, TaskId::new(1), permit), Ruling::Deny);
        facts.same = Err(());
        assert_eq!(judge(&facts, TaskId::new(1), permit), Ruling::Unjudged);
        facts.opener = None;
        assert_eq!(judge(&facts, TaskId::new(1), permit), Ruling::Unjudged);
    }
}
