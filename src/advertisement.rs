use forest_cid;
use forest_ipld as ipld;
use forest_ipld::Ipld;
use ipld_blockstore::BlockStore;
use serde::{Deserialize, Serialize};

/// Represents the advertisement we are going to broadcast too the indexers.
/// This is defined at: <https://github.com/filecoin-project/storetheindex/blob/main/api/v0/ingest/schema/schema.ipldsch>
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Advertisement {
    /// CID of the Previous advertisement
    pub PreviousID: Option<ipld::Ipld>,
    /// Provider ID of the advertisement.
    pub Provider: String,
    /// list of multiaddr strings, to use for content retrieval.
    pub Addresses: Vec<String>,
    /// Advertisement signature.
    pub Signature: ipld::Ipld,
    /// Entries with a link to the list of CIDs
    pub Entries: Option<ipld::Ipld>,
    /// Context ID for entries.
    pub ContextID: ipld::Ipld,
    /// Serialized v0.Metadata for all entries in advertisement.
    pub Metadata: ipld::Ipld,
    /// Is Removal or Put?
    pub IsRm: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct AdvertisementBuilder {
    pub(crate) ad: Advertisement,
    pub(crate) entries_link: Option<ipld::Ipld>,
}

impl AdvertisementBuilder {
    pub(crate) fn link_entries(
        &mut self,
        chunk_builder: &dyn EntryChunkBuilder,
        entries: Vec<Ipld>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.entries_link = Some(chunk_builder.link_entries(self.entries_link.take(), entries)?);
        Ok(())
    }

    pub(crate) fn build(mut self) -> Advertisement {
        self.ad.Entries = self.entries_link;
        self.ad
    }
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct EntryChunk {
    // A vec of multihashes represented as Ipld::Bytes
    pub Entries: Vec<Ipld>,
    pub Next: Option<ipld::Ipld>,
}

// #[derive(Serialize, Deserialize)]
// struct EntryChunkBuilder<BS> {
//     next_link: Option<ipld::Ipld>,
//     blockstore: BS,
// }

pub(crate) trait EntryChunkBuilder {
    fn link_entries(
        &self,
        entries_link: Option<ipld::Ipld>,
        entries: Vec<Ipld>,
    ) -> Result<ipld::Ipld, Box<dyn std::error::Error>>;
}

impl<BS: BlockStore> EntryChunkBuilder for BS {
    fn link_entries(
        &self,
        entries_link: Option<ipld::Ipld>,
        entries: Vec<Ipld>,
    ) -> Result<ipld::Ipld, Box<dyn std::error::Error>> {
        let chunk = EntryChunk {
            Entries: entries,
            Next: entries_link,
        };
        let cid = self.put(&chunk, forest_cid::Code::Blake2b256)?;
        return Ok(ipld::Ipld::Link(cid));
    }
}

#[cfg(test)]
mod tests {
    use forest_db::MemoryDB;
    use forest_encoding::Cbor;
    use multihash::MultihashDigest;

    use super::*;
    #[test]
    fn test_parse_adv_from_go() -> Result<(), Box<dyn std::error::Error>> {
        // This encoded string came from an advertisment in Go.
        let ad_encoded = "qGRJc1Jt9GdFbnRyaWVz2CpWAAGpAhIQ03tRRXFZ/6VnhQKtTEHbsGhNZXRhZGF0YVgmgIDAARIg1L/zUOdGp1RKwC5NGvLWJVsfhDPXAKNM10zYk7bHfxNoUHJvdmlkZXJ4NDEyRDNLb29XSEh6U2VLYVk4eHVaVnprTGJLRmZ2TmdQUGVLaEZCR3JNYk56Ym01YWtwcXVpQWRkcmVzc2VzgXcvaXA0LzEyNy4wLjAuMS90Y3AvOTk5OWlDb250ZXh0SURPdGVzdC1jb250ZXh0LWlkaVNpZ25hdHVyZVkBEgokCAESIG8VgXCbt7HvAw0hDbGOOwuhx3b7pl2M2q0FQVFC0Yn4EhsvaW5kZXhlci9pbmdlc3QvYWRTaWduYXR1cmUaigEShwEBqQISENN7UUVxWf+lZ4UCrUxB27AxMkQzS29vV0hIelNlS2FZOHh1WlZ6a0xiS0Zmdk5nUFBlS2hGQkdyTWJOemJtNWFrcHF1L2lwNC8xMjcuMC4wLjEvdGNwLzk5OTmAgMABEiDUv/NQ50anVErALk0a8tYlWx+EM9cAo0zXTNiTtsd/EwAqQDeGRd/deuiSXLBwTcsVxqrCWJ9XAiLk74KH0AKIh/NE9XArZITOdHugWcpBd4vjXP4wRge+NNn01q9IQBLdtQ9qUHJldmlvdXNJRPY=";
        let ad_bytes = base64::decode(ad_encoded).unwrap();
        let ad: Advertisement = forest_encoding::from_slice(&ad_bytes)?;
        println!("Ad is {:?}", ad);
        println!("Ad bytelen is {:?}", ad_bytes.len());
        println!(
            "Ad sig bytelen is {:?}",
            forest_encoding::to_vec(&ad.Signature)?.len()
        );
        Ok(())
    }

    #[test]
    fn test_build_entries() {
        let bs = MemoryDB::default();

        let mh = multihash::Code::Blake2b256.digest(b"Hello world");
        println!("Multihash: {:?}", mh);

        let chunk_link = bs.link_entries(None, vec![Ipld::Bytes(mh.into())]).unwrap();
        let serialized = chunk_link.marshal_cbor().unwrap();
        println!("serialized {:?}", serialized);
    }
}
