use forest_cid;
use forest_ipld as ipld;
use forest_ipld::Ipld;
use ipld_blockstore::BlockStore;
use libp2p::core::{signed_envelope, SignedEnvelope};
use libp2p::identity::Keypair;
use multihash::MultihashDigest;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const AD_SIGNATURE_CODEC: &'static str = "/indexer/ingest/adSignature";
const AD_SIGNATURE_DOMAIN: &'static str = "indexer";

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

impl Advertisement {
    fn sign(&self, signing_key: Keypair) -> Result<SignedEnvelope, AdSigError> {
        Ok(SignedEnvelope::new(
            signing_key,
            AD_SIGNATURE_DOMAIN.into(),
            AD_SIGNATURE_CODEC.into(),
            self.sig_payload()?,
        )
        .map_err(AdSigError::SigningError)?)
    }

    pub fn sig_payload(&self) -> Result<Vec<u8>, AdSigError> {
        let mut previous_id_bytes = match &self.PreviousID {
            Some(Ipld::Link(link)) => Ok(link.to_bytes()),
            None => Ok(vec![]),
            _ => Err(AdSigError::InvalidPreviousID),
        }?;

        let mut entrychunk_link_bytes: Vec<u8> = match &self.Entries {
            Some(Ipld::Link(link)) => Ok(link.to_bytes()),
            None => Ok(vec![]),
            _ => Err(AdSigError::InvalidEntryChunkLink),
        }?;

        let metadata = match &self.Metadata {
            Ipld::Bytes(b) => Ok(b),
            _ => Err(AdSigError::InvalidMetadata),
        }?;

        // let is_rm_payload = if self.IsRm == Ipld::Bool(true) {
        let is_rm_payload = if self.IsRm { [1] } else { [0] };

        let mut payload: Vec<u8> = Vec::with_capacity(
            previous_id_bytes.len()
                + entrychunk_link_bytes.len()
                + self.Provider.len()
                + self.Addresses.iter().map(|s| s.len()).sum::<usize>()
                + metadata.len()
                + is_rm_payload.len(),
        );

        payload.append(&mut previous_id_bytes);
        payload.append(&mut entrychunk_link_bytes);
        payload.extend_from_slice(&self.Provider.as_bytes());
        self.Addresses
            .iter()
            .for_each(|s| payload.extend_from_slice(&s.as_bytes()));
        payload.extend_from_slice(metadata);
        payload.extend_from_slice(&is_rm_payload);

        Ok(multihash::Code::Sha2_256.digest(&payload).to_bytes())
    }

    pub(crate) fn verify_sig(&self) -> Result<(), AdSigError> {
        let payload = self.sig_payload()?;
        let signed_env_bytes = match &self.Signature {
            Ipld::Bytes(b) => b,
            _ => return Err(AdSigError::MissingSig),
        };

        let signed_env = SignedEnvelope::from_protobuf_encoding(&signed_env_bytes)
            .map_err(AdSigError::DecodingError)?;

        let signed_payload = signed_env
            .payload(AD_SIGNATURE_DOMAIN.into(), AD_SIGNATURE_CODEC.as_bytes())
            .map_err(AdSigError::ReadPayloadError)?;

        if signed_payload != payload {
            Err(AdSigError::PayloadDidNotMatch)?;
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct AdvertisementBuilder {
    pub(crate) ad: Advertisement,
    pub(crate) entries_link: Option<ipld::Ipld>,
}

#[derive(Debug, Error)]
pub enum AdSigError {
    #[error("Invalid Previous ID")]
    InvalidPreviousID,
    #[error("Invalid Entry Chunk Link")]
    InvalidEntryChunkLink,
    #[error("Invalid Metadata")]
    InvalidMetadata,
    #[error("Missing Signature")]
    MissingSig,
    #[error("Failed to sign advertisement: {0}")]
    SigningError(libp2p::identity::error::SigningError),
    #[error("Failed to decode sig: {0}")]
    DecodingError(signed_envelope::DecodingError),
    #[error("Failed to read signed payload: {0}")]
    ReadPayloadError(signed_envelope::ReadPayloadError),
    #[error("Payload did not match expected")]
    PayloadDidNotMatch,
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

    pub(crate) fn build(mut self, signing_key: Keypair) -> Result<Advertisement, AdSigError> {
        self.ad.Entries = self.entries_link;
        let sig = self.ad.sign(signing_key)?;
        self.ad.Signature = Ipld::Bytes(sig.into_protobuf_encoding());
        Ok(self.ad)
    }
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct EntryChunk {
    // A vec of multihashes represented as Ipld::Bytes
    pub Entries: Vec<Ipld>,
    pub Next: Option<ipld::Ipld>,
}

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
        let test_cases = vec![
        // isRm = false. Has previous ID.
        "qGRJc1Jt9GdFbnRyaWVz2CpWAAGpAhIQ9zEJmkwOPzLfq3XOpiYhbGhNZXRhZGF0YVgmgIDAARIgjRQocOtZoqJMyZdFPsDWR2iIVbXrWSzOSgKgYgQf8GZoUHJvdmlkZXJ4NDEyRDNLb29XS1J5elZXVzZDaEZqUWpLNG1pQ3R5ODVOaXk0OXRwUFY5NVhkS3UxQmN2TUFpQWRkcmVzc2VzgXcvaXA0LzEyNy4wLjAuMS90Y3AvOTk5OWlDb250ZXh0SURRdGVzdC1jb250ZXh0LWlkLTFpU2lnbmF0dXJlWKkKJAgBEiCO2QQggCyDtB5Kf6lM5fBXkuqL/z16Y1cuXHNFTq71HRIbL2luZGV4ZXIvaW5nZXN0L2FkU2lnbmF0dXJlGiISIOyp+iaVcXZxBsclNCjscl3KKTUmi+qm7rwJb6JhvckDKkCabk3tCMdhqv0timkDfL4dGZMnB8EpwC/Xv2z0HWMKKFAcKH3Awsrwotgt9tl3VNPrBEpMVrYFCyusM6aAKJQMalByZXZpb3VzSUTYKlYAAakCEhCFimyUPpafqUqal21lVWft",
        // isRm = true
        "qGRJc1Jt9WdFbnRyaWVz2CpVAAFVEhDjsMRCmPwcFJr79MiZb7kkaE1ldGFkYXRhWCaAgMABEiCNFChw61miokzJl0U+wNZHaIhVtetZLM5KAqBiBB/wZmhQcm92aWRlcng0MTJEM0tvb1dLUnl6VldXNkNoRmpRaks0bWlDdHk4NU5peTQ5dHBQVjk1WGRLdTFCY3ZNQWlBZGRyZXNzZXOBdy9pcDQvMTI3LjAuMC4xL3RjcC85OTk5aUNvbnRleHRJRFF0ZXN0LWNvbnRleHQtaWQtMGlTaWduYXR1cmVYqQokCAESII7ZBCCALIO0Hkp/qUzl8FeS6ov/PXpjVy5cc0VOrvUdEhsvaW5kZXhlci9pbmdlc3QvYWRTaWduYXR1cmUaIhIgUkVeGezGmLcyD6GmC6wBdXV+nzk4vvkZUZE3jrWc2JMqQBqaGzNIMpM6LVwV8GQ318mW5DVSrjln12FM6qRmvpWAapC9epycqJ3CQpB37omUeLKSEDBOpkyiBC/Wqiznnw1qUHJldmlvdXNJRNgqVgABqQISEHHI8ycOIDYfZrCPLUpWpsI"
        ];
        for ad_encoded in test_cases {
            let ad_bytes = base64::decode(ad_encoded).unwrap();
            let ad: Advertisement = forest_encoding::from_slice(&ad_bytes)?;
            ad.verify_sig()?;
        }

        Ok(())
    }

    #[test]
    fn test_roundtrip_sig() {
        let bs = MemoryDB::default();
        let mh = multihash::Code::Blake2b256.digest(b"Hello world");

        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let provider = libp2p::PeerId::from_public_key(&keypair.public());

        let mut ad_builder = AdvertisementBuilder {
            entries_link: None,
            ad: Advertisement {
                Entries: None,
                Signature: Ipld::Bytes(vec![]),
                Addresses: vec!["/ip4/1.1.1.1/tcp/1234".into()],
                ContextID: Ipld::Bytes("asdf".into()),
                IsRm: false,
                Metadata: Ipld::Bytes("Some meta".into()),
                PreviousID: None,
                Provider: provider.to_base58(),
            },
        };

        ad_builder
            .link_entries(&bs, vec![Ipld::Bytes(mh.into())])
            .unwrap();

        let ad = ad_builder.build(keypair.clone()).expect("Signing failed");
        ad.verify_sig().expect("Signature verification failed");
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
