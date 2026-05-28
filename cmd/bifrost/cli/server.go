package cli

import (
	"context"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/maximhq/bifrost/core/schemas"
	"github.com/spf13/cobra"

	"github.com/kooshapari/bifrost-extensions/plugins/intelligentrouter"
	"github.com/kooshapari/bifrost-extensions/plugins/learning"
	"github.com/kooshapari/bifrost-extensions/plugins/smartfallback"
)

var (
	port     int
	host     string
	plugins  []string
	logLevel string
)

var serverCmd = &cobra.Command{
	Use:   "server",
	Short: "Start the Bifrost server",
	Long:  `Start the Bifrost LLM gateway server with configured plugins and providers.`,
	RunE:  runServer,
}

func init() {
	serverCmd.Flags().IntVarP(&port, "port", "p", 8080, "Server port")
	serverCmd.Flags().StringVarP(&host, "host", "h", "0.0.0.0", "Server host")
	serverCmd.Flags().StringSliceVarP(&plugins, "plugins", "P", []string{"router", "learning", "fallback"}, "Plugins to load")
	serverCmd.Flags().StringVarP(&logLevel, "log-level", "l", "info", "Log level (debug, info, warn, error)")
}

func runServer(cmd *cobra.Command, args []string) error {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Setup structured logger
	var level slog.Level
	switch logLevel {
	case "debug":
		level = slog.LevelDebug
	case "warn":
		level = slog.LevelWarn
	case "error":
		level = slog.LevelError
	default:
		level = slog.LevelInfo
	}
	logger := slog.New(slog.NewTextHandler(os.Stdout, &slog.HandlerOptions{
		Level: level,
	})).With("component", "bifrost-server")

	// Handle shutdown signals
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		sig := <-sigChan
		logger.Info("Shutting down server", "signal", sig.String())
		cancel()
	}()

	// Create plugins
	var pluginList []schemas.Plugin
	for _, p := range plugins {
		switch p {
		case "router":
			pluginList = append(pluginList, intelligentrouter.New(intelligentrouter.DefaultConfig()))
		case "learning":
			pluginList = append(pluginList, learning.New(learning.DefaultConfig()))
		case "fallback":
			pluginList = append(pluginList, smartfallback.New(smartfallback.DefaultConfig()))
		}
	}

	logger.Info("Bifrost server started",
		"host", host,
		"port", port,
		"plugins", plugins,
	)

	// Wait for shutdown
	<-ctx.Done()
	logger.Info("Shutdown complete")

	return nil
}

