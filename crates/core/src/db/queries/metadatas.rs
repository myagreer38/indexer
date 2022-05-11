//! Query utilities for looking up  metadatas

use tera::{Context as TeraContext, Tera};
use diesel::{
    pg::Pg,
    debug_query,
    prelude::*,
    serialize::ToSql,
    sql_types::{Array, Text, Integer, Nullable, Bool},
};

use crate::{
    prelude::*,
    db::{
        any,
        models::{Nft, NftActivity, ListedNft},
        not,
        tables::{
            attributes, bid_receipts, current_metadata_owners, listing_receipts,
            metadata_collection_keys, metadata_creators, metadata_jsons, metadatas,
        },
        Connection,
    },
    error::prelude::*,
};
/// Format for incoming filters on attributes
#[derive(Debug)]
pub struct AttributeFilter {
    /// name of trait
    pub trait_type: String,
    /// array of trait values
    pub values: Vec<String>,
}

/// List query options
#[derive(Debug)]
pub struct ListQueryOptions {
    /// nft owners
    pub owners: Option<Vec<String>>,
    /// auction houses
    pub auction_houses: Option<Vec<String>>,
    /// nft creators
    pub creators: Option<Vec<String>>,
    /// offerers who provided offers on nft
    pub offerers: Option<Vec<String>>,
    /// nft attributes
    pub attributes: Option<Vec<AttributeFilter>>,
    /// nfts listed for sale
    pub listed: Option<bool>,
    /// nft in a specific colleciton
    pub collection: Option<String>,
    /// limit to apply to query
    pub limit: i64,
    /// offset to apply to query
    pub offset: i64,
}

/// The column set for an NFT
pub type NftColumns = (
    metadatas::address,
    metadatas::name,
    metadatas::seller_fee_basis_points,
    metadatas::mint_address,
    metadatas::primary_sale_happened,
    metadatas::uri,
    metadata_jsons::description,
    metadata_jsons::image,
    metadata_jsons::category,
    metadata_jsons::model,
);

const METADATAS_QUERY_TEMPLATE: &str = r"
SELECT
    metadatas_address,
    metadatas.name,
    metadatas.seller_fee_basis_points,
    metadatas.mint_address,
    metadatas.primary_sale_happend,
    metadatas.uri,
    metadatas_jsons.description,
    metadatas_jsons.image,
    metadatas_jsons.category,
    metadata_jsons.model,
    listing_receipt.price
    FROM metadata_jsons
        LEFT JOIN LATERAL (
            SELECT * 
                FROM listing_receipts 
                WHERE listing_receipts.metadata = metadata_jsons.metadata_address
                    AND curren_metadata_owners.owner_address = listing_receipts.seller
                    AND listing_receipts.canceled_at IS NULL
                    AND listing_receipts.purchase_receipt IS NULL
                    AND ($4 IS NULL OR listing_receipts.auction_house = ANY($4))
                ORDER BY listing_receipts.price DESC
        ) listing_receipts ON TRUE
        INNER JOIN metadatas ON (metadatas.address = metadata_jsons.metadata_address)
        INNER JOIN metadata_creators ON (metadatas.address = metadata_creators.metadata_address)
        INNER JOIN current_metadata_owners ON (metadatas.mint_address = current_metadata_owners.mint_address)
        {% if offerers %}
        INNER JOIN LATERAL (
            SELECT * FROM bid_receipts 
                WHERE bid_receipts.metadata = metadata_jsons.metadata_address
                AND ($4 IS NULL OR bid_receipts.auction_house = ANY($4))
                AND bid_receipts.buyer = ANY($5)
                AND bid_receipts.canceled_at IS NULL
                AND bid_receipts.purchase_receipt IS NULL
        ) bid_receipts ON TRUE
        {% endif %}
        {% set attribue_filter_params = 5 %}
        {% for attribute_filter in attribute_filters %}
            INNER JOIN LATERAL (
                SELECT * FROM attributes
                    WHERE attributes.metadata_address = metadata_jsons.metadata_address
                    AND attributes.trait_type = ${{ attribue_filter_params + 1 }}
                    AND attributes.value = ANY(${{ attribue_filter_params + 2 }})
            ) attributes_{{ attribute_filter.index }} ON TRUE
        {% set attribue_filter_params = attribute_filter_params + 2 %}
        {% endfor %}
    WHERE ($3 IS NULL OR metadata_creators.creator_address = ANY($3))
        AND metadata_creators.verified
    {% if listed %}
        AND listing_receipts.price IS NOT NULL
    {% endif %}
    ORDER BY listing_receipts.price DESC, metadatas.name ASC
    LIMIT $1
    OFFSET $2;
-- $1: limit::integer
-- $2: offset::integer
-- $3: creators::text[]
-- $4: auction houses::text[]
-- $5: offerers::text[]
{% if attribue_filter_params > 5 %}
{% for i in range(end=attribue_filter_params, start=6) %}
-- $i: {% if i is even %}attribue trait {{ i }}::text{% else %}attribue values {{ i }}::text[]{% endif %}
{% endfor %}
{% endif %}
";

/// Handles queries for NFTs
///
/// # Errors
/// returns an error when the underlying queries throw an error
#[allow(clippy::too_many_lines)]
pub fn list(
    conn: &Connection,
        owners: impl ToSql<Nullable<Array<Text>>, Pg>,
        creators: impl ToSql<Nullable<Array<Text>>, Pg>,
        auction_houses: impl ToSql<Nullable<Array<Text>>, Pg>,
        offerers: impl ToSql<Nullable<Array<Text>>, Pg>,
        listed: impl ToSql<Nullable<Bool>, Pg>,
        limit: impl ToSql<Integer, Pg>,
        offset: impl ToSql<Integer, Pg>,
) -> Result<Vec<ListedNft>> {
    let mut context = TeraContext::new();
    let attribute_filters: Vec<String> = vec![];

    context.insert("listed", &listed);
    context.insert("attribute_filters", &attribute_filters);
    
    context.insert("offerers", &offerers);

    let query_string = Tera::one_off(METADATAS_QUERY_TEMPLATE, &context, true)?;

    debug!("The template string: {:?}", query_string);

    let query = diesel::sql_query(query_string)
        .bind(limit)
        .bind(offset)
        .bind(creators)
        .bind(auction_houses)
        .bind(offerers);

    let sql = debug_query::<Pg, _>(&query);
    let result = sql.to_string().replace("\"", "");

    debug!("{:?}", result);
    let rows: Vec<ListedNft>  = query.load(conn).context("Failed to load nft(s)")?;

    Ok(rows)
}

const ACTIVITES_QUERY: &str = r"
    SELECT address, metadata, auction_house, price, auction_house, created_at, array[seller::text] as wallets, 'listing' as activity_type
        FROM listing_receipts WHERE metadata = ANY($1)
    UNION
    SELECT address, metadata, auction_house, price, auction_house, created_at, array[seller::text, buyer::text] as wallets, 'purchase' as activity_type
        FROM purchase_receipts WHERE metadata = ANY($1)
    ORDER BY created_at DESC;
 -- $1: addresses::text[]";

/// Load listing and sales activity for nfts
///
/// # Errors
/// This function fails if the underlying SQL query returns an error
pub fn activities(
    conn: &Connection,
    addresses: impl ToSql<Array<Text>, Pg>,
) -> Result<Vec<NftActivity>> {
    diesel::sql_query(ACTIVITES_QUERY)
        .bind(addresses)
        .load(conn)
        .context("Failed to load nft(s) activities")
}
