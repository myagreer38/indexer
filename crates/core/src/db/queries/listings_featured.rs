//! Query utilities for global stats
use diesel::{
    prelude::*,
    sql_types::{Integer, Text},
};

use crate::{
    db::{models::FeaturedListing, Connection},
    error::prelude::*,
};

static HOLAPLEX_MARKETPLACES_ADDRESS: &str = "9SvsTjqk3YoicaYnC4VW1f8QAN9ku7QCCk6AyfUdzc9t";

// diesel doesnt have a functional group_by or order_by with dynamic columns (that isnt overly complicated)
const FEATURED_LISTINGS_QUERY: &str = r"
select
    listing_receipts.address,
    listing_receipts.metadata,
    listing_receipts.created_at

from listing_receipts

where
    listing_receipts.auction_house = '$1'
    and listing_receipts.purchase_receipt is null
    and listing_receipts.canceled_at is null
    and listing_receipts.price is not null

order by listing_receipts.created_at desc
limit $2
offset $3
-- $1: auction_house::integer
-- $2: limit::integer
-- $3: creator::text";

/// Return a list of featured, active listings
///
/// # Errors
/// This function fails if the underlying query fails to execute.
pub fn list(conn: &Connection, limit: i32, offset: Option<i32>) -> Result<Vec<FeaturedListing>> {
    let offset: i32 = offset.unwrap_or(0);

    diesel::sql_query(FEATURED_LISTINGS_QUERY)
        .bind::<Text, _>(HOLAPLEX_MARKETPLACES_ADDRESS)
        .bind::<Integer, _>(limit)
        .bind::<Integer, _>(offset)
        .load(conn)
        .context("Failed to load featured listings")
}
