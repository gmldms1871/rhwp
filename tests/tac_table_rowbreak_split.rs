//! treat_as_char(글자처럼 취급) 표가 채우기로 쪽을 넘길 때 행 단위로 분할되는지 검증.
//!
//! `paginate_table_control`은 표가 현재 쪽에 안 들어가면 (a) TAC 표는 통째로 다음 쪽 이동,
//! (b) 비-TAC 표는 `split_table_rows`로 분기했다. 이 때문에 TAC 표는 `page_break=RowBreak`
//! 여도 행 분할 경로에 도달하지 못하고, **빈 쪽에도 안 들어가는 크기**가 되면 통째 이동이
//! 무의미해져 쪽 밖으로 그려졌다(값은 들어가는데 인쇄하면 뒷부분이 안 보임).
//!
//! samples/table-001.hwp: 1쪽, 19×9 TAC 표(쪽나눔=RowBreak), 표 높이 336px / 본문 876.9px.
//! 여러 셀에 텍스트를 채워 표를 본문 높이보다 크게 만든 뒤 쪽이 늘어나는지 본다.

use rhwp::document_core::DocumentCore;
use rhwp::model::control::Control;

const SAMPLE: &str = "samples/table-001.hwp";

fn load_sample() -> DocumentCore {
    let bytes = std::fs::read(SAMPLE).unwrap_or_else(|e| panic!("read {SAMPLE}: {e}"));
    DocumentCore::from_bytes(&bytes).unwrap_or_else(|e| panic!("parse {SAMPLE}: {e}"))
}

/// (문단 인덱스, 컨트롤 인덱스) — 구역 0의 첫 TAC 표.
fn find_tac_table(core: &DocumentCore) -> (usize, usize) {
    for (para_idx, para) in core.document().sections[0].paragraphs.iter().enumerate() {
        for (ctrl_idx, ctrl) in para.controls.iter().enumerate() {
            if matches!(ctrl, Control::Table(t) if t.common.treat_as_char) {
                return (para_idx, ctrl_idx);
            }
        }
    }
    panic!("{SAMPLE}: TAC 표를 찾지 못함");
}

fn cell_count(core: &DocumentCore, para_idx: usize, ctrl_idx: usize) -> usize {
    match &core.document().sections[0].paragraphs[para_idx].controls[ctrl_idx] {
        Control::Table(t) => t.cells.len(),
        other => panic!("표가 아님: {other:?}"),
    }
}

/// (표 선언 높이, 대상 셀 높이, 호스트 문단 첫 LINE_SEG 높이)
fn stored_metrics(
    core: &DocumentCore,
    para_idx: usize,
    ctrl_idx: usize,
    cell_idx: usize,
) -> (u32, u32, i32) {
    let para = &core.document().sections[0].paragraphs[para_idx];
    let host_lh = para
        .line_segs
        .first()
        .map(|seg| seg.line_height)
        .unwrap_or(0);
    match &para.controls[ctrl_idx] {
        Control::Table(t) => (t.common.height, t.cells[cell_idx].height, host_lh),
        other => panic!("표가 아님: {other:?}"),
    }
}

/// 셀을 채우면 저장 모델(셀 높이 → 표 선언 높이 → 호스트 LINE_SEG)이 함께 커져야 한다.
/// 이 값들이 그대로면 내보낸 파일이 "값은 들어갔는데 높이는 원래대로"인 상태가 되어,
/// 다시 여는 쪽(rhwp든 한컴이든)이 표를 원래 크기로 조판한다.
#[test]
fn cell_fill_grows_stored_table_height() {
    let mut core = load_sample();
    let (para_idx, ctrl_idx) = find_tac_table(&core);
    let cell_idx = 0;

    let (before_table_h, before_cell_h, before_host_lh) =
        stored_metrics(&core, para_idx, ctrl_idx, cell_idx);

    let text = "가나다라마바사아자차".repeat(12);
    core.insert_text_in_cell_native(0, para_idx, ctrl_idx, cell_idx, 0, 0, &text)
        .expect("셀 채우기");

    core.sync_stored_table_heights_for_export();

    let (after_table_h, after_cell_h, after_host_lh) =
        stored_metrics(&core, para_idx, ctrl_idx, cell_idx);

    assert!(
        after_cell_h > before_cell_h,
        "셀 높이가 그대로다: {before_cell_h} → {after_cell_h}"
    );
    assert!(
        after_table_h > before_table_h,
        "표 선언 높이가 그대로다: {before_table_h} → {after_table_h}"
    );
    assert!(
        after_host_lh > before_host_lh,
        "호스트 문단 LINE_SEG 높이가 그대로다: {before_host_lh} → {after_host_lh}"
    );
}

/// FreeForm 실제 흐름: 채운다 → **파일로 내보낸다** → 다른 프로세스가 그 파일을 연다.
/// 커진 높이가 직렬화(표는 `raw_ctrl_data` 로 재기록됨)를 통과하지 못하면 재오픈 시
/// 원래 크기로 돌아가 같은 증상이 난다 — changelog 2026-07-20 (3) 참고.
#[test]
fn filled_table_keeps_grown_height_across_serialize_roundtrip() {
    let mut core = load_sample();
    let (para_idx, ctrl_idx) = find_tac_table(&core);

    let text = "가나다라마바사아자차".repeat(12);
    let cells = cell_count(&core, para_idx, ctrl_idx);
    for cell_idx in (0..cells).step_by(9) {
        core.insert_text_in_cell_native(0, para_idx, ctrl_idx, cell_idx, 0, 0, &text)
            .unwrap_or_else(|e| panic!("셀 {cell_idx} 채우기 실패: {e:?}"));
    }
    core.sync_stored_table_heights_for_export();
    let pages_before_save = core.page_count();

    let bytes = rhwp::serializer::cfb_writer::serialize_hwp(core.document())
        .unwrap_or_else(|e| panic!("직렬화 실패: {e}"));
    let mut reopened = DocumentCore::from_bytes(&bytes).expect("재오픈");
    reopened.repaginate_if_needed();

    assert!(
        reopened.page_count() > 1,
        "재오픈하니 다시 1쪽이다 (저장 전 {pages_before_save}쪽) — 커진 높이가 직렬화에서 유실됨"
    );
}

/// HWPX 는 코드 경로가 다르다 — `update_ctrl_dimensions` 의 `raw_ctrl_data` 패치가
/// 건너뛰어지고(HWPX 파스 문서는 raw 가 없음) 직렬화기가 `common` 에서 합성한다.
/// 실무 양식 재현이 HWPX 이므로 이 경로도 함께 고정한다.
#[test]
fn hwpx_filled_table_keeps_grown_height_across_serialize_roundtrip() {
    const HWPX: &str = "samples/issue_2148_degenerate_cell_vpos.hwpx";
    let bytes = std::fs::read(HWPX).unwrap_or_else(|e| panic!("read {HWPX}: {e}"));
    let mut core = DocumentCore::from_bytes(&bytes).unwrap_or_else(|e| panic!("parse {HWPX}: {e}"));
    let (para_idx, ctrl_idx) = find_tac_table(&core);

    let text = "가나다라마바사아자차".repeat(12);
    let cells = cell_count(&core, para_idx, ctrl_idx);
    for cell_idx in 0..cells {
        core.insert_text_in_cell_native(0, para_idx, ctrl_idx, cell_idx, 0, 0, &text)
            .unwrap_or_else(|e| panic!("셀 {cell_idx} 채우기 실패: {e:?}"));
    }
    core.sync_stored_table_heights_for_export();
    let (table_h, _, host_lh) = stored_metrics(&core, para_idx, ctrl_idx, 0);
    let pages_before_save = core.page_count();

    let out = rhwp::serializer::hwpx::serialize_hwpx(core.document())
        .unwrap_or_else(|e| panic!("HWPX 직렬화 실패: {e}"));
    let mut reopened = DocumentCore::from_bytes(&out).expect("재오픈");
    reopened.repaginate_if_needed();
    let (re_table_h, _, re_host_lh) = stored_metrics(&reopened, para_idx, ctrl_idx, 0);

    assert_eq!(
        re_table_h, table_h,
        "HWPX 왕복에서 표 선언 높이가 유실됨 ({table_h} → {re_table_h})"
    );
    assert_eq!(
        re_host_lh, host_lh,
        "HWPX 왕복에서 호스트 LINE_SEG 높이가 유실됨 ({host_lh} → {re_host_lh})"
    );
    assert_eq!(
        reopened.page_count(),
        pages_before_save,
        "HWPX 재오픈 쪽 수가 저장 전과 다르다"
    );
}

#[test]
fn tac_rowbreak_table_splits_pages_after_cell_fill() {
    let mut core = load_sample();
    let (para_idx, ctrl_idx) = find_tac_table(&core);

    // DocumentCore::from_bytes 는 페이지네이션을 하지 않는다(무상태 CLI 규약).
    core.repaginate_if_needed();
    assert_eq!(
        core.page_count(),
        1,
        "{SAMPLE} 원본은 1쪽이어야 한다 (전제 확인)"
    );

    // 본문(876.9px)을 넘기도록 여러 행의 셀을 채운다.
    // 한 셀에 몰아넣으면 그 행 하나가 쪽보다 커져 행 분할로도 못 담으므로,
    // 실제 양식 채우기와 같은 형태(여러 행에 분산)로 만든다.
    let text = "가나다라마바사아자차".repeat(12);
    let cells = cell_count(&core, para_idx, ctrl_idx);
    for cell_idx in (0..cells).step_by(9) {
        core.insert_text_in_cell_native(0, para_idx, ctrl_idx, cell_idx, 0, 0, &text)
            .unwrap_or_else(|e| panic!("셀 {cell_idx} 채우기 실패: {e:?}"));
    }

    core.sync_stored_table_heights_for_export();
    assert!(
        core.page_count() > 1,
        "채운 표가 본문 높이를 넘었는데 쪽이 늘지 않았다 (page_count={})",
        core.page_count()
    );
}
