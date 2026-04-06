package contextfolding

import (
	"context"

	"github.com/kooshapari/bifrost-extensions/slm"
	"github.com/maximhq/bifrost/core/schemas"
)

// toChatMessages converts []schemas.Message to []schemas.ChatMessage
func toChatMessages(messages []schemas.Message) []schemas.ChatMessage {
	result := make([]schemas.ChatMessage, len(messages))
	for i, msg := range messages {
		result[i] = schemas.ChatMessage{
			Role:    msg.Role,
			Content: msg.Content,
		}
	}
	return result
}

// toMessages converts []schemas.ChatMessage to []schemas.Message
func toMessages(messages []schemas.ChatMessage) []schemas.Message {
	result := make([]schemas.Message, len(messages))
	for i, msg := range messages {
		result[i] = schemas.Message{
			Role:    msg.Role,
			Content: msg.Content,
		}
	}
	return result
}

// getStrategy determines the context strategy to use
func (cf *ContextFolding) getStrategy(ctx context.Context) ContextStrategy {
	// Check if strategy was set by router
	if strategy, ok := ctx.Value("context_strategy").(ContextStrategy); ok {
		return strategy
	}
	return cf.config.DefaultStrategy
}

// calculateBudget calculates available token budget
func (cf *ContextFolding) calculateBudget(req *schemas.BifrostRequest) int {
	budget := cf.config.MaxContextTokens
	budget -= cf.config.ReserveOutputTokens
	budget -= cf.config.SystemPromptTokens

	// Subtract current message tokens
	if req.ChatRequest != nil && len(req.ChatRequest.Messages) > 0 {
		for _, msg := range req.ChatRequest.Messages {
			// Rough token estimate: ~4 chars per token
			budget -= len(msg.Content) / 4
		}
	}

	if budget < 0 {
		budget = 0
	}
	return budget
}

// foldContext applies context folding based on strategy
func (cf *ContextFolding) foldContext(
	ctx context.Context,
	req *schemas.BifrostRequest,
	strategy ContextStrategy,
	budget int,
) *schemas.BifrostRequest {
	if req.ChatRequest == nil || len(req.ChatRequest.Messages) == 0 {
		return req
	}

	modifiedReq := *req
	var messages []schemas.ChatMessage
	switch strategy {
	case StrategyRawOnly:
		// Keep all messages as-is, truncate if needed
		messages = cf.truncateToFit(req.ChatRequest.Messages, budget)

	case StrategyShortSummary:
		// Summarize old messages, keep recent raw
		messages = cf.summarizeOld(ctx, req.ChatRequest.Messages, budget, "short")

	case StrategyMediumSummary:
		messages = cf.summarizeOld(ctx, req.ChatRequest.Messages, budget, "medium")

	case StrategyFullSummary:
		messages = cf.summarizeOld(ctx, req.ChatRequest.Messages, budget, "long")

	case StrategyMediumWithRawOnDemand:
		// Use medium summaries but keep raw for important messages
		messages = cf.adaptiveFold(ctx, req.ChatRequest.Messages, budget)

	case StrategyAdaptive:
		// Dynamically choose based on content
		messages = cf.adaptiveFold(ctx, req.ChatRequest.Messages, budget)

	default:
		messages = req.ChatRequest.Messages
	}

	// Copy and update request
	newChatReq := *req.ChatRequest
	newChatReq.Messages = messages
	modifiedReq.ChatRequest = &newChatReq

	return &modifiedReq
}

// truncateToFit keeps most recent messages that fit in budget
func (cf *ContextFolding) truncateToFit(messages []schemas.ChatMessage, budget int) []schemas.ChatMessage {
	result := make([]schemas.ChatMessage, 0)
	usedTokens := 0

	// Keep messages from the end (most recent)
	for i := len(messages) - 1; i >= 0; i-- {
		msg := messages[i]
		tokens := cf.estimateTokens(&msg)

		if usedTokens+tokens > budget && len(result) > 0 {
			break
		}
		result = append([]schemas.ChatMessage{msg}, result...)
		usedTokens += tokens
	}

	return result
}

// summarizeOld summarizes older messages and keeps recent ones raw
func (cf *ContextFolding) summarizeOld(
	ctx context.Context,
	messages []schemas.ChatMessage,
	budget int,
	length string,
) []schemas.ChatMessage {
	if len(messages) <= 3 {
		return messages
	}

	// Keep last 3 messages raw
	recentCount := 3
	recent := messages[len(messages)-recentCount:]
	old := messages[:len(messages)-recentCount]

	// Calculate tokens for recent messages
	recentTokens := 0
	for _, msg := range recent {
		recentTokens += cf.estimateTokens(&msg)
	}
	_ = recentTokens // will use for budget calculation later

	// Summarize old messages if we have SLM client
	var summaryMsg *schemas.ChatMessage
	if cf.slmClients != nil && cf.slmClients.Summarizer != nil {
		oldContent := cf.messagesToText(old)
		if resp, err := cf.slmClients.Summarize(ctx, &slm.SummarizeRequest{
			Text:          oldContent,
			Mode:          "conversation_segment",
			DesiredLength: length,
		}); err == nil {
			content := "[Previous conversation summary]\n" + resp.Summary
			summaryMsg = &schemas.ChatMessage{
				Role:    string(schemas.ChatMessageRoleSystem),
				Content: content,
			}
		}
	}

	// Build result
	result := make([]schemas.ChatMessage, 0, recentCount+1)
	if summaryMsg != nil {
		result = append(result, *summaryMsg)
	}
	result = append(result, recent...)

	return result
}

// adaptiveFold uses importance-based folding
func (cf *ContextFolding) adaptiveFold(
	ctx context.Context,
	messages []schemas.ChatMessage,
	budget int,
) []schemas.ChatMessage {
	// For now, use medium summary strategy
	// TODO: Implement importance scoring
	return cf.summarizeOld(ctx, messages, budget, "medium")
}

