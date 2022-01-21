# Rust Index Provider Example

This is an example implementation of an Index Provider that advertises
CIDs to [storetheindex]. For a full featured implementation, take a look at the
Go reference implementation of the [index-provider].

This is meant to run as a separate service, and clients (such as lotus) can call
this service to build an [Advertisement], and then publish it.

This implementation only uses the http transport for serving Advertisements.
This is an implementation detail, so don't worry if that doesn't make sense.
(The go implementation can use both the http transport and the Datatransfer
transport)

## An overview

There are a couple players here. I'll use specific examples to make it clearer,
but note that some of these can be generalized.

1. Filecoin Storage Provider – Hosts data for folks and proves it via the
   Filecoin Network Chain. Aka Storage Provider.
1. Indexer (aka [storetheindex]) – A service that can answer the question:
   "Given this CID, who has a copy?". This is little more than a lookup table.
1. Index Provider – A service that runs alongside a Storage Provider and tells
   the Indexer what content this storage provider has.

The Index Provider serves as the interface between the storage provider and the
indexer. It can be used from within [Lotus] so that the publishing of new data
happens automatically. But it can also happen separately.

The Index Provider sends updates to the Indexer via a series of [Advertisement]
messages. Each message references a previous advertisement so that as a whole it
forms an advertisement chain. The state of the indexer is basically a function
of consuming this chain from the initial Advertisement to the latest one.


## How this works

This will start an http server with a couple of endpoints:

On a public facing port, it serves:
`GET /head` → Return the current latest advertisement cid.
`GET /<cid>` → Return the bytes for a cid.

On a separate private port, this server will also serve:
`POST /create` → Returns a temporary id that represents this work-in-progress
advertisement. Takes as input required fields for the advertisement (except for
entries) encoded as an dag-cbor representation of Advertisement.
`POST /adv/<tempID>/entryChunk` → Add a entrychunk to this advertisement.
Repeated calls will link the chunks together. This is to let the caller avoid
allocating space for all the entries at once. The body should be a dag-cbor
representation of a list of multihashes.
`POST /adv/<tempID>/publish` → Builds the advertisement and puts it in the local
datastore. It is now available to be requested by the indexer. Returns the cid
of this advertisement. After this is called, `Get /head` will also return this
cid.

## TODO
* Sign the advertisements

## Questions

Can we use openapi here?


## How the Indexer learns about new Advertisements

Two options:
1. Polling.
2. Pubsub messages.
  * This may not be implemented yet, but should be relatively straightforward.
    You simply publish the head cid to a specific gossipsub channel.


[Advertisement]: https://github.com/filecoin-project/storetheindex/blob/main/api/v0/ingest/schema/schema.ipldsch
[index-provider]: https://github.com/filecoin-project/index-provider/
[storetheindex]: https://github.com/filecoin-project/storetheindex
[Lotus]: https://github.com/filecoin-project/lotus