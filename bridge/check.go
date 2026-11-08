package main

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/config"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/probe"
)

// check dials the given addresses (by default every address `pair` would hand to the plugin) with
// this bridge's token and reports, for each, whether the plugin would get through.
func check(urls []string) error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	if len(urls) == 0 {
		urls = pairURLs(cfg)
	}
	ok := 0
	for _, u := range urls {
		ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
		r, err := probe.Probe(ctx, u, cfg.Token)
		cancel()
		if err != nil {
			fmt.Printf("NG  %s\n      %v\n", u, err)
			continue
		}
		ok++
		fmt.Printf("OK  %s\n      bridge %q, version %s, protocol %d, %d ms\n", u, r.Bridge, r.Version, r.Protocol, r.Elapsed.Milliseconds())
	}
	if ok == 0 {
		return errors.New("no address could be reached")
	}
	return nil
}
