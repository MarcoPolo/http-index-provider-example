package main

import (
	"bytes"
	"flag"
	"fmt"
	"io"
	"net/http"
	"strconv"

	v0 "github.com/filecoin-project/storetheindex/api/v0"
	schema "github.com/filecoin-project/storetheindex/api/v0/ingest/schema"
	"github.com/ipld/go-car/v2"
	"github.com/ipld/go-car/v2/index"
	"github.com/ipld/go-ipld-prime"
	"github.com/ipld/go-ipld-prime/codec/dagcbor"
	"github.com/ipld/go-ipld-prime/datamodel"
	"github.com/ipld/go-ipld-prime/linking"
	cidlink "github.com/ipld/go-ipld-prime/linking/cid"

	"github.com/multiformats/go-multicodec"
	"github.com/multiformats/go-multihash"
)

const chunkSize = 10

func noErr(err error) {
	if err != nil {
		panic("Unexpected error: " + err.Error())
	}
}

func main() {
	carPath := flag.String("carFile", "", "a path to a car file")
	providerID := flag.String("providerID", "", "a provider ID")
	flag.Parse()

	cr, err := car.OpenReader(*carPath)
	noErr(err)

	idxReader, err := iterableIndex(cr)
	noErr(err)

	var mhChunk = make([]multihash.Multihash, 0, chunkSize)

	i := importerHelper{}
	i.startAdPublish([]byte(*carPath), []byte(""), *providerID, []string{"/ip4/1.1.1.1/tcp/1234"}, false)

	idxReader.ForEach(func(m multihash.Multihash, offset uint64) error {
		// fmt.Printf("%s %d\n", m, u)
		mhChunk = append(mhChunk, m)
		if len(mhChunk) == chunkSize {
			mhChunk, err = i.publishEntryChunk(mhChunk)
			noErr(err)
		}
		return nil
	})

	if len(mhChunk) > 0 {
		mhChunk, err = i.publishEntryChunk(mhChunk)
		noErr(err)
	}

	// idxReader.ForEach(f)

	err = i.publishAd()
	noErr(err)

	fmt.Println("Hello, world", *carPath)
}

type importerHelper struct {
	tempAdId int64
}

func newLsys() ipld.LinkSystem {
	lsys := cidlink.DefaultLinkSystem()
	lsys.StorageWriteOpener = func(lc linking.LinkContext) (io.Writer, linking.BlockWriteCommitter, error) {
		var out bytes.Buffer
		return &out, func(l datamodel.Link) error {
			fmt.Println("Commit ", l, len(out.Bytes()))
			return nil
		}, nil
	}

	lsys.StorageReadOpener = func(lc linking.LinkContext, l datamodel.Link) (io.Reader, error) {
		return nil, nil
	}

	return lsys

}

func (i *importerHelper) publishAd() error {
	c := http.Client{}

	resp, err := c.Post(fmt.Sprintf("http://localhost:8071/adv/%d/publish", i.tempAdId), "application/octet-stream", bytes.NewReader([]byte{}))
	if err != nil {
		return err
	}
	fmt.Println("Publish ad Resp ", resp)

	return nil
}

func (i *importerHelper) publishEntryChunk(mhChunk []multihash.Multihash) ([]multihash.Multihash, error) {
	lsys := newLsys()
	_, ec, err := schema.NewLinkedListOfMhs(lsys, mhChunk, nil)
	if err != nil {
		return nil, err
	}

	c := http.Client{}

	var out bytes.Buffer
	dagcbor.Encode(&ec.Entries, &out)
	resp, err := c.Post(fmt.Sprintf("http://localhost:8071/adv/%d/entryChunk", i.tempAdId), "application/octet-stream", bytes.NewReader(out.Bytes()))
	if err != nil {
		return nil, err
	}
	fmt.Println("Resp ", resp)

	return mhChunk[:0], nil
}

// type AdProto struct {
// 	PreviousID: basicnode.Node,
// }

func (i *importerHelper) startAdPublish(contextID []byte, metadata []byte, provider string, addrs []string, isRm bool) error {
	// http.C
	// PreviousID: None,
	// Provider: "12D3KooWHHzSeKaY8xuZVzkLbKFfvNgPPeKhFBGrMbNzbm5akpqu".into(),
	// Addresses: vec!["/ip4/127.0.0.1/tcp/9999".into()],
	// Signature: Ipld::Bytes(vec![]),
	// Entries: None,
	// Metadata: Ipld::Bytes(vec![]),
	// ContextID: Ipld::Bytes("some-context".into()),
	// IsRm: false,
	lsys := newLsys()
	md := v0.Metadata{
		ProtocolID: 0x300010,
		Data:       metadata,
	}
	ad, _, err := schema.NewAdvertisementWithFakeSig(lsys, nil, nil, schema.NoEntries, contextID, md, isRm, provider, addrs)
	if err != nil {
		return err
	}

	c := http.Client{}

	var out bytes.Buffer
	dagcbor.Encode(ad, &out)
	resp, err := c.Post("http://localhost:8071/create", "application/octet-stream", bytes.NewReader(out.Bytes()))
	if err != nil {
		return err
	}
	fmt.Println("Resp from create", resp)
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	fmt.Println("Body ", string(body))
	tempID, err := strconv.Atoi(string(body))
	if err != nil {
		return err
	}

	i.tempAdId = int64(tempID)
	fmt.Println("TEMP ID is", tempID)

	return nil
}

func iterableIndex(cr *car.Reader) (index.IterableIndex, error) {

	idxReader := cr.IndexReader()
	if idxReader == nil {
		// Missing index; generate it.
		return generateIterableIndex(cr)
	}
	idx, err := index.ReadFrom(idxReader)
	if err != nil {
		return nil, err
	}
	codec := idx.Codec()
	if codec != multicodec.CarMultihashIndexSorted {
		return generateIterableIndex(cr)
	}
	itIdx, ok := idx.(index.IterableIndex)
	if !ok {
		// Though technically possible, this should not happen, since the expectation is that
		// multicodec.CarMultihashIndexSorted implements the index.IterableIndex interface.
		// Regardless, defensively check this and re-generate as needed in case go-car library
		// changes this expectation.
		return generateIterableIndex(cr)
	}
	return itIdx, nil
}

func generateIterableIndex(cr *car.Reader) (index.IterableIndex, error) {
	idx := index.NewMultihashSorted()
	if err := car.LoadIndex(idx, cr.DataReader()); err != nil {
		return nil, err
	}
	return idx, nil
}
