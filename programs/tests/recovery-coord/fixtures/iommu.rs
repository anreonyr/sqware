use crate::iommu_api::{Call, Said, Wire};
use super::{action::Action, peer::Fixture, script::Script};

impl Fixture for Call {
    type Request = Wire;
    type Response = Said;
    fn call(script: &mut Script, request: Wire) -> Result<Said, &'static str> {
        script.iommu.push(request);
        let (action, okay) = match request {
            Wire::Acquire(_) => (Action::IommuAcquire, script.acquire_ok),
            Wire::Publish(_) => (Action::IommuPublish, script.publish_ok),
            Wire::Invalidate(_) => (Action::IommuInvalidate, script.invalidate_ok),
            Wire::Quiesce(_) => (Action::IommuQuiesce, script.quiesce_ok),
            Wire::Release(_) => (Action::IommuRelease, script.iommu_release_ok),
        };
        script.action(action);
        if !okay { return Err("injected IOMMU transport error after effect"); }
        Ok(Said {
            status: if matches!(request, Wire::Acquire(_)) { script.iommu_status } else { 0 },
            pending: u8::from(matches!(request, Wire::Publish(_)) && script.publish_pending),
            iova: if matches!(request, Wire::Acquire(_)) { script.iova } else { 0 },
            ..Said::NONE
        })
    }
}
