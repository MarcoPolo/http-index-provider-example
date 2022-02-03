mod advertisement;
mod signed_head;

use advertisement::{Advertisement, AdvertisementBuilder};
use async_std::{
    self,
    sync::{Arc, RwLock},
};
use forest_cid::Cid;
use forest_db::MemoryDB;
use forest_ipld::Ipld;
use ipld_blockstore::BlockStore;
use libp2p::{futures::future::join, identity::Keypair};
use rand;
use rand::Rng;
use serde_json::Value;
use signed_head::SignedHead;
use std::collections::HashMap;
use tide::StatusCode;
use tide::{self, utils::After, Body, Response};

async fn head<BS>(r: tide::Request<Provider<BS>>) -> tide::Result<Body> {
    if let Some(head) = *r.state().head.read().await {
        let signed_head = SignedHead::new(&r.state().keypair, head)?;
        Body::from_json(&signed_head)
    } else {
        tide::Result::Err(tide::Error::from_str(tide::StatusCode::NotFound, "No head"))
    }
}

async fn block<BS: BlockStore>(req: tide::Request<Provider<BS>>) -> tide::Result<Body> {
    let cid: Cid = req.param("cid")?.parse()?;
    let bs = req.state().blockstore.read().await;
    let res = bs.get_bytes(&cid);
    match res {
        Ok(Some(bytes)) => Ok(Body::from_bytes(bytes)),
        Ok(None) => tide::Result::Err(tide::Error::from_str(
            tide::StatusCode::NotFound,
            "block not found",
        ))?,
        Err(e) => tide::Result::Err(tide::Error::from_str(
            tide::StatusCode::InternalServerError,
            format!("{}", e),
        ))?,
    }
}

async fn create<BS>(mut r: tide::Request<Provider<BS>>) -> tide::Result<Value> {
    let id: i64 = rand::thread_rng().gen();
    let ad: Advertisement = forest_encoding::from_slice(&r.body_bytes().await?)?;
    let builder = AdvertisementBuilder {
        ad,
        entries_link: None,
    };

    let mut temp_ads = r.state().temp_ads.write().await;
    temp_ads.insert(id, builder);

    Ok(id.into())
}

async fn add_chunk<BS: BlockStore>(mut r: tide::Request<Provider<BS>>) -> tide::Result<StatusCode> {
    let id: i64 = r.param("id")?.parse()?;
    let entries: Vec<Ipld> = forest_encoding::from_slice(&r.body_bytes().await?)?;
    let mut temp_ads = r.state().temp_ads.write().await;
    if let Some(ad_builder) = temp_ads.get_mut(&id) {
        let bs = r.state().blockstore.write().await;
        ad_builder.link_entries(&*bs, entries).map_err(|e| {
            tide::Error::from_str(tide::StatusCode::InternalServerError, format!("{}", e))
        })?;
        return Ok(StatusCode::Ok);
    }

    tide::Result::Err(tide::Error::from_str(
        tide::StatusCode::NotFound,
        "Temporary ad not found from given id",
    ))
}

async fn publish_ad<BS: BlockStore>(r: tide::Request<Provider<BS>>) -> tide::Result<String> {
    let id: i64 = r.param("id")?.parse()?;
    let mut head = r.state().head.write().await;
    let keypair = r.state().keypair.as_ref().clone();
    let current_head = head.take();
    let mut temp_ads = r.state().temp_ads.write().await;
    if let Some(ad_builder) = temp_ads.remove(&id) {
        let bs = r.state().blockstore.write().await;
        let mut ad = ad_builder.build(keypair)?;
        ad.PreviousID = current_head.map(|h| forest_ipld::Ipld::Link(h));
        let ipld_node = forest_ipld::to_ipld(ad)?;

        let cid = bs
            .put(&ipld_node, forest_cid::Code::Blake2b256)
            .map_err(|e| {
                tide::Error::from_str(tide::StatusCode::InternalServerError, format!("{}", e))
            })?;
        *head = Some(cid);
        return Ok(cid.to_string());
    }

    tide::Result::Err(tide::Error::from_str(
        tide::StatusCode::NotFound,
        "Temporary ad not found",
    ))
}

#[derive(Clone)]
struct Provider<BS> {
    head: Arc<RwLock<Option<Cid>>>,
    keypair: Arc<Keypair>,
    blockstore: Arc<RwLock<BS>>,
    temp_ads: Arc<RwLock<HashMap<i64, AdvertisementBuilder>>>,
}

fn main() -> Result<(), std::io::Error> {
    let provider = Provider {
        blockstore: Arc::new(RwLock::new(MemoryDB::default())),
        head: Arc::new(RwLock::new(None)),
        keypair: Arc::new(Keypair::generate_ed25519()),
        temp_ads: Arc::new(RwLock::new(HashMap::new())),
    };
    let mut app = tide::with_state(provider.clone());
    let mut admin_app = tide::with_state(provider.clone());

    async_std::task::block_on(async {
        app.at("/head").get(head);
        app.at("/:cid").get(block);
        app.with(After(|res: Response| async {
            if let Some(err) = res.error() {
                println!("Server error: {:?}", err)
            }
            Ok(res)
        }));
        admin_app.with(After(|res: Response| async {
            if let Some(err) = res.error() {
                println!("Server error: {:?}", err)
            }
            Ok(res)
        }));

        admin_app.at("/create").post(create);
        admin_app.at("/adv/:id/entryChunk").post(add_chunk);
        admin_app.at("/adv/:id/publish").post(publish_ad);

        let (app_res, admin_res) =
            join(app.listen("0.0.0.0:8070"), admin_app.listen("0.0.0.0:8071")).await;
        app_res.expect("failed to start server");
        admin_res.expect("failed to start admin server");
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use forest_encoding::from_slice;
    use forest_ipld::Ipld;
    use multihash::MultihashDigest;
    use tide_testing::TideTestingExt;

    #[test]
    fn test_create_ad() -> Result<(), Box<dyn std::error::Error>> {
        async_std::task::block_on(async {
            let provider = Provider {
                blockstore: Arc::new(RwLock::new(MemoryDB::default())),
                head: Arc::new(RwLock::new(None)),
                keypair: Arc::new(Keypair::generate_ed25519()),
                temp_ads: Arc::new(RwLock::new(HashMap::new())),
            };

            let mut app = tide::with_state(provider.clone());
            app.at("/head").get(head);
            app.at("/:cid").get(block);
            app.at("/create").post(create);
            app.at("/adv/:id/entryChunk").post(add_chunk);
            app.at("/adv/:id/publish").post(publish_ad);

            app.with(After(|res: Response| async {
                if let Some(err) = res.error() {
                    println!("Server error: {:?}", err)
                }
                Ok(res)
            }));

            // We didn't pass anythign in so this should fail
            assert_eq!(
                app.post("/create").send().await?.status(),
                tide::StatusCode::InternalServerError
            );

            // Create an advertisement
            let ad = Advertisement {
                PreviousID: None,
                Provider: "12D3KooWHHzSeKaY8xuZVzkLbKFfvNgPPeKhFBGrMbNzbm5akpqu".into(),
                Addresses: vec!["/ip4/127.0.0.1/tcp/9999".into()],
                Signature: Ipld::Bytes(vec![]),
                Entries: None,
                Metadata: Ipld::Bytes(vec![]),
                ContextID: Ipld::Bytes("some-context".into()),
                IsRm: false,
            };
            let ad_bytes = forest_encoding::to_vec(&ad)?;

            let mut resp = app.post("/create").body_bytes(ad_bytes).send().await?;
            assert_eq!(resp.status(), tide::StatusCode::Ok);
            let id = resp.body_string().await?.parse::<i64>().unwrap();
            println!("id {:?}", id);

            // Put entries in that advertisement
            let mut entries: Vec<Ipld> = vec![];
            let count = 10;

            for i in 0..count {
                let b = Into::<i32>::into(i).to_ne_bytes();
                let mh = multihash::Code::Blake2b256.digest(&b);
                entries.push(Ipld::Bytes(mh.to_bytes()))
            }

            let entries_bytes = forest_encoding::to_vec(&entries)?;

            let resp = app
                .post(format!("/adv/{}/entryChunk", id))
                .body_bytes(entries_bytes)
                .send()
                .await?;

            assert_eq!(resp.status(), tide::StatusCode::Ok);

            // Publish the advertisement
            let mut resp = app.post(format!("/adv/{}/publish", id)).send().await?;
            assert_eq!(resp.status(), tide::StatusCode::Ok);
            let published_ad_cid = forest_cid::Cid::from_str(&resp.body_string().await?)?;

            // Check that the head has been updated
            let signed_head: SignedHead = app.get("/head").recv_json().await?;
            assert_eq!(signed_head.open()?.1, published_ad_cid);

            // Check that we can fetch the advertisement via http
            let mut resp = app.get(format!("/{}", published_ad_cid)).send().await?;
            assert_eq!(resp.status(), tide::StatusCode::Ok);

            let ad_bytes = resp.body_bytes().await?;
            let ad: Advertisement = from_slice(&ad_bytes)?;
            println!("ad {:?}", ad);

            Ok(())
        })
    }
}
