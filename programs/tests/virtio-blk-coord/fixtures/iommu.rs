use crate::iommu_api::{Call, Said, Wire};
use super::{action::Action, peer::Fixture, script::Script};

impl Fixture for Call {
    type Request = Wire;
    type Response = Said;

    fn call(script: &mut Script, request: Wire) -> Result<Said, &'static str> {
        script.iommu.push(request);
        let (action, okay) = match request {
            Wire::Establish(_) => panic!("domain established before enrollment"),
            Wire::Acquire(_) => (Action::IommuAcquire, script.acquire_ok),
            Wire::Publish(_) => (Action::IommuPublish, script.publish_ok),
            Wire::Invalidate(_) => (Action::IommuInvalidate, script.invalidate_ok),
            Wire::Quiesce(_) => (Action::IommuQuiesce, script.quiesce_ok),
            Wire::Release(_) => (Action::IommuRelease, script.iommu_release_ok),
        };
        script.action(action);
        if !okay { return Err("injected IOMMU transport error after effect"); }
        let (status, pending, engine, key, iova) = match request {
            Wire::Acquire(frame) => (
                script.iommu_status,
                0,
                frame.engine,
                frame.key,
                script.iova_for(frame.key),
            ),
            Wire::Publish(frame) => (
                0,
                u8::from(script.publish_pending),
                frame.engine,
                frame.key,
                0,
            ),
            Wire::Invalidate(frame) | Wire::Quiesce(frame) | Wire::Release(frame) => (
                0, 0, frame.engine, frame.key, 0,
            ),
            Wire::Establish(_) => unreachable!(),
        };
        Ok(Said {
            status,
            pending,
            engine,
            key,
            iova,
            bytes: if iova != 0 { 6 * env::PAGE_SIZE as u64 } else { 0 },
            ..Said::NONE
        })
    }

    fn held(script: &Script, request: &Wire) -> bool {
        match request {
            Wire::Acquire(frame) => script.hold_iommu == Some((crate::iommu_api::ACQUIRE, frame.serial)),
            Wire::Publish(frame) => script.hold_iommu == Some((crate::iommu_api::PUBLISH, frame.serial)),
            Wire::Invalidate(frame) => script.hold_iommu == Some((crate::iommu_api::INVALIDATE, frame.serial)),
            Wire::Quiesce(frame) => script.hold_iommu == Some((crate::iommu_api::QUIESCE, frame.serial)),
            Wire::Release(frame) => script.hold_iommu == Some((crate::iommu_api::RELEASE, frame.serial)),
            Wire::Establish(_) => false,
        }
    }
}
