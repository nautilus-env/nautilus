//! The entry points that turn sqlx rows into Nautilus rows, one row, one batch
//! or one stream at a time.

use sqlx::postgres::PgRow;

use super::decode::decode_row_with_plan;
use super::decode_plan::PgColumnPlan;
use crate::error::Result;
use crate::row_stream::RowStream;
use crate::Row;

/// Stream type for PostgreSQL query results.
///
/// A thin alias for the shared [`RowStream`] type.
pub type PgRowStream<'conn> = RowStream<'conn>;

/// Decode a sqlx `PgRow` into a Nautilus `Row`.
///
/// Standalone single-row entry point: column classification is paid per call.
/// Multi-row paths should prefer [`decode_rows`] or [`streaming_decoder`],
/// which classify each column once per statement.
pub(crate) fn decode_row_internal(row: PgRow) -> Result<Row> {
    let plan = PgColumnPlan::for_row(&row);
    decode_row_with_plan(&plan, &row)
}

/// Decode a batch of rows produced by a single statement, classifying the
/// column types once (on the first row) instead of once per cell.
pub(crate) fn decode_rows(rows: &[PgRow]) -> Result<Vec<Row>> {
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let plan = PgColumnPlan::for_row(first);
    rows.iter()
        .map(|row| decode_row_with_plan(&plan, row))
        .collect()
}

/// Stateful decoder for streaming paths: builds the column plan from the
/// first row that arrives and reuses it for every subsequent row of the
/// statement.
pub(crate) fn streaming_decoder() -> impl FnMut(PgRow) -> Result<Row> + Send + 'static {
    let mut plan: Option<PgColumnPlan> = None;
    move |row| {
        let plan = plan.get_or_insert_with(|| PgColumnPlan::for_row(&row));
        decode_row_with_plan(plan, &row)
    }
}
