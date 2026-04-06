package toolrouter

import (
	"context"

	"github.com/kooshapari/bifrost-extensions/slm"
	"github.com/maximhq/bifrost/core/schemas"
)

// getToolProfile gets tool profile from context
func (tr *ToolRouter) getToolProfile(ctx context.Context) slm.ToolProfile {
	if profile, ok := ctx.Value(toolProfileKey).(slm.ToolProfile); ok {
		return profile
	}
	return slm.ToolProfile{}
}

// filterTools filters and prioritizes tools based on profile
func (tr *ToolRouter) filterTools(
	ctx context.Context,
	req *schemas.BifrostRequest,
	profile slm.ToolProfile,
) *schemas.BifrostRequest {
	// Check if tools are in Params
	toolsRaw, ok := req.Params["tools"]
	if !ok || toolsRaw == nil {
		return req
	}

	tools, ok := toolsRaw.([]schemas.ChatTool)
	if !ok || len(tools) == 0 {
		return req
	}

	// If no profile, try to get one from SLM
	if len(profile.Preferred) == 0 && tr.slmClients != nil {
		profile = tr.classifyTools(ctx, req)
	}

	// Filter and reorder tools
	filteredTools := tr.applyProfile(tools, profile)

	// Limit number of tools
	if len(filteredTools) > tr.config.MaxToolsPerRequest {
		filteredTools = filteredTools[:tr.config.MaxToolsPerRequest]
	}

	// Create modified request - copy and update
	modifiedReq := *req
	newParams := make(map[string]interface{})
	for k, v := range req.Params {
		newParams[k] = v
	}
	newParams["tools"] = filteredTools
	modifiedReq.Params = newParams

	return &modifiedReq
}

// classifyTools uses SLM to classify tools for this request
func (tr *ToolRouter) classifyTools(ctx context.Context, req *schemas.BifrostRequest) slm.ToolProfile {
	if tr.slmClients == nil || tr.slmClients.Router == nil {
		return slm.ToolProfile{}
	}

	// Build tool names list
	var toolNames []string
	toolsRaw, ok := req.Params["tools"]
	if ok {
		if tools, ok := toolsRaw.([]schemas.ChatTool); ok {
			for _, tool := range tools {
				if tool.Function.Name != "" {
					toolNames = append(toolNames, tool.Function.Name)
				}
			}
		}
	}

	// Get last user message for context
	var userMessage string
	if req.ChatRequest != nil && len(req.ChatRequest.Messages) > 0 {
		for i := len(req.ChatRequest.Messages) - 1; i >= 0; i-- {
			msg := req.ChatRequest.Messages[i]
			if msg.Role == "user" {
				userMessage = msg.Content
				break
			}
		}
	}

	// Call router SLM to classify - use ClassifyRequest with conversation
	messages := make([]slm.Message, 0, 1)
	messages = append(messages, slm.Message{Role: "user", Content: userMessage})

	resp, err := tr.slmClients.Classify(ctx, &slm.ClassifyRequest{
		Conversation: messages,
		UserMessage:  userMessage,
	})
	if err != nil {
		return slm.ToolProfile{}
	}

	// Build profile from classification - use Preferred field
	profile := slm.ToolProfile{
		Preferred: toolNames, // For now, include all tools
	}

	// Use role from classification if available
	if resp.Role != "" {
		profile.AllowedCategories = []string{resp.Role}
	}

	return profile
}

// applyProfile reorders tools based on profile
func (tr *ToolRouter) applyProfile(tools []schemas.ChatTool, profile slm.ToolProfile) []schemas.ChatTool {
	if len(profile.Preferred) == 0 {
		return tools
	}

	// Create priority map from Preferred field
	priority := make(map[string]int)
	for i, name := range profile.Preferred {
		priority[name] = i
	}

	// Separate prioritized and other tools
	prioritized := make([]schemas.ChatTool, 0)
	other := make([]schemas.ChatTool, 0)

	for _, tool := range tools {
		if tool.Function.Name != "" {
			if _, ok := priority[tool.Function.Name]; ok {
				prioritized = append(prioritized, tool)
			} else if tr.config.FallbackToAllTools {
				other = append(other, tool)
			}
		}
	}

	// Combine: prioritized first, then others
	result := make([]schemas.ChatTool, 0, len(prioritized)+len(other))
	result = append(result, prioritized...)
	result = append(result, other...)

	return result
}

// trackToolUsage tracks which tools were used for learning
func (tr *ToolRouter) trackToolUsage(ctx context.Context, resp *schemas.BifrostResponse) {
	// TODO: Extract tool calls from response and record metrics
}

