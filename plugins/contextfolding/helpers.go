package contextfolding

import (
	"context"
	"strings"

	"github.com/kooshapari/bifrost-extensions/slm"
	"github.com/maximhq/bifrost/core/schemas"
)

// estimateTokens estimates tokens in a message
func (cf *ContextFolding) estimateTokens(msg *schemas.ChatMessage) int {
	return len(msg.Content) / 4
}

// messagesToText converts messages to plain text
func (cf *ContextFolding) messagesToText(messages []schemas.ChatMessage) string {
	var sb strings.Builder
	for _, msg := range messages {
		sb.WriteString(string(msg.Role))
		sb.WriteString(": ")
		sb.WriteString(msg.Content)
		sb.WriteString("\n\n")
	}
	return sb.String()
}

// summarizeResponse summarizes a response for future context
// summarizeResponse summarizes a response for future context
func (cf *ContextFolding) summarizeResponse(ctx context.Context, resp *schemas.BifrostResponse) {
	if cf.slmClients == nil || cf.slmClients.Summarizer == nil {
		return
	}
	if resp == nil || resp.ChatResponse == nil {
		return
	}
	
	// Get the content from the response
	content := resp.ChatResponse.Content()
	if content == "" {
		return
	}

	cf.slmClients.Summarize(ctx, &slm.SummarizeRequest{
		Text: content,
		Mode: "response",
	})
}
// RetrieveRelevantContext retrieves relevant context from database
func (cf *ContextFolding) RetrieveRelevantContext(
	ctx context.Context,
	embedding []float32,
	sessionID string,
	limit int,
) ([]string, error) {
	if cf.queries == nil {
		return nil, nil
	}

	// This would use vector similarity search
	// For now, return empty
	// TODO: Implement once we have embedding generation
	return nil, nil
}

