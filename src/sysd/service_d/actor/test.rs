//! ServiceD actor 集成测试 (service 集合, job 队列 与 reset-failed).

use crate::{
    jsonl::{SdReq, SdReqUnitOp, SdRes},
    sysd::test_util::*,
};

#[tokio::test]
async fn reset_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move { is_failed(s, "t.service").await })
    })
    .await;
    assert!(is_failed(&sysd, "t.service").await);
    svc_reset_failed(&sysd, Some("t.service")).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move { is_failed(s, "t.service").await })
    })
    .await;
    assert!(is_failed(&sysd, "t.service").await);
    svc_reset_failed(&sysd, None).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn reset_failed_never_run_silent_success() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_reset_failed(&sysd, Some("t.service")).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn reset_failed_unknown_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(svc_reset_failed(&sysd, Some("nope.service")).await.is_err());
}

#[tokio::test]
async fn handle_start_uses_job_queue() {
    let text = unit_conf(&run_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    match handle(&sysd, &SdReq::Unit(unit("t.service", SdReqUnitOp::Start))).await {
        SdRes::Ok(_) => {}
        other => panic!("unexpected response: {:?}", other),
    }
    assert!(is_active(&sysd, "t.service").await);
}
