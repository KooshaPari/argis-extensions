// Package main provides the entry point for the enhanced Bifrost server
// with intelligent routing, learning, and smart fallback capabilities.
package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/maximhq/bifrost/core/schemas"
	bifrost "github.com/maximhq/bifrost/core/schemas"

	"github.com/kooshapari/bifrost-extensions/plugins/intelligentrouter"
	"github.com/kooshapari/bifrost-extensions/plugins/learning"
	"github.com/kooshapari/bifrost-extensions/plugins/smartfallback"
)

func main() {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Handle shutdown signals
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigChan
		log.Println("Shutting down...")
		cancel()
	}()

	// Create enhanced account with configured providers
	acct := schemas.NewEnhancedAccount(nil)

	// Create plugins
	routerPlugin := intelligentrouter.New(intelligentrouter.DefaultConfig())
	learningPlugin := learning.New(learning.DefaultConfig())
	fallbackPlugin := smartfallback.New(smartfallback.DefaultConfig())

	// Start learning plugin background processes
	learningPlugin.Start(ctx)

	// Initialize Bifrost with plugins
	bf, err := bifrost.Init(ctx, schemas.BifrostConfig{
		Account: acct,
		Plugins: []schemas.Plugin{
			routerPlugin,
			learningPlugin,
			fallbackPlugin,
		},
		Logger:          bifrost.NewDefaultLogger(schemas.LogLevelInfo),
		InitialPoolSize: 100,
	})
	if err != nil {
		log.Fatalf("Failed to initialize Bifrost: %v", err)
	}

	fmt.Println("Enhanced Bifrost initialized successfully!")
	fmt.Println("Plugins loaded:")
	fmt.Printf("  - %s (intelligent routing)\n", routerPlugin.GetName())
	fmt.Printf("  - %s (performance learning)\n", learningPlugin.GetName())
	fmt.Printf("  - %s (smart fallback)\n", fallbackPlugin.GetName())

	// Wait for shutdown
	<-ctx.Done()

	// Cleanup
	bf.Shutdown()

	fmt.Println("Shutdown complete")
}

