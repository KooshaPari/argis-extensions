package resolvers

import (
	"context"
	"fmt"

	"github.com/kooshapari/bifrost-extensions/api/graphql/model"
)

// mutationResolver handles mutation operations
type mutationResolver struct {
	*Resolver
}

// UpdateModelStatus updates a model's availability status
func (r *mutationResolver) UpdateModelStatus(ctx context.Context, id string, available bool, reason *string) (*model.Model, error) {
	if r.models == nil {
		return nil, fmt.Errorf("model store not configured")
	}

	updated, err := r.models.UpdateModelStatus(ctx, id, available)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to update model status",
			"model_id", id,
			"available", available,
			"error", err,
		)
		return nil, err
	}

	// Publish availability event
	updatedModel, ok := updated.(*model.Model)
	if !ok {
		return nil, fmt.Errorf("failed to cast updated model")
	}
	r.PublishModelAvailability(&model.ModelAvailabilityEvent{
		Model:     updatedModel,
		Available: available,
		Reason:    reason,
	})

	return updatedModel, nil
}

// CreateModel creates a new model in the registry
func (r *mutationResolver) CreateModel(ctx context.Context, input model.CreateModelInput) (*model.Model, error) {
	if r.models == nil {
		return nil, fmt.Errorf("model store not configured")
	}
	created, err := r.models.CreateModel(ctx, input)
	if err != nil {
		return nil, err
	}
	createdModel, ok := created.(*model.Model)
	if !ok {
		return nil, fmt.Errorf("failed to cast created model")
	}
	return createdModel, nil
}

// UpdatePolicy updates an existing policy
func (r *mutationResolver) UpdatePolicy(ctx context.Context, id string, input model.PolicyInput) (*model.Policy, error) {
	if r.policies == nil {
		return nil, fmt.Errorf("policy store not configured")
	}
	
	policyInterface, err := r.policies.UpdatePolicy(ctx, id, input)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to update policy",
			"policy_id", id,
			"error", err,
		)
		return nil, err
	}

	policy, ok := policyInterface.(*model.Policy)
	if !ok {
		return nil, fmt.Errorf("failed to cast updated policy")
	}
	return policy, nil
}

// ActivatePolicy activates a policy
func (r *mutationResolver) ActivatePolicy(ctx context.Context, id string) (*model.Policy, error) {
	if r.policies == nil {
		return nil, fmt.Errorf("policy store not configured")
	}
	
	policyInterface, err := r.policies.ActivatePolicy(ctx, id)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to activate policy",
			"policy_id", id,
			"error", err,
		)
		return nil, err
	}

	policy, ok := policyInterface.(*model.Policy)
	if !ok {
		return nil, fmt.Errorf("failed to cast activated policy")
	}
	r.logger.InfoContext(ctx, "policy activated", "policy_id", id)
	return policy, nil
}

// DeactivatePolicy deactivates a policy
func (r *mutationResolver) DeactivatePolicy(ctx context.Context, id string) (*model.Policy, error) {
	if r.policies == nil {
		return nil, fmt.Errorf("policy store not configured")
	}
	
	policyInterface, err := r.policies.DeactivatePolicy(ctx, id)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to deactivate policy",
			"policy_id", id,
			"error", err,
		)
		return nil, err
	}

	policy, ok := policyInterface.(*model.Policy)
	if !ok {
		return nil, fmt.Errorf("failed to cast deactivated policy")
	}
	r.logger.InfoContext(ctx, "policy deactivated", "policy_id", id)
	return policy, nil
}

// CreateBenchmark creates a new benchmark run
func (r *mutationResolver) CreateBenchmark(ctx context.Context, input model.BenchmarkInput) (*model.Benchmark, error) {
	if r.benchmarks == nil {
		return nil, fmt.Errorf("benchmark store not configured")
	}
	
	benchmarkInterface, err := r.benchmarks.CreateBenchmark(ctx, input)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to create benchmark",
			"name", input.Name,
			"error", err,
		)
		return nil, err
	}

	benchmark, ok := benchmarkInterface.(*model.Benchmark)
	if !ok {
		return nil, fmt.Errorf("failed to cast created benchmark")
	}

	r.logger.InfoContext(ctx, "benchmark created",
		"benchmark_id", benchmark.ID,
		"name", benchmark.Name,
		"models", len(input.ModelIds),
	)

	return benchmark, nil
}

// RefreshProviderToken refreshes OAuth token for a provider account
func (r *mutationResolver) RefreshProviderToken(ctx context.Context, providerID string) (*model.Account, error) {
	if r.providers == nil {
		return nil, fmt.Errorf("provider store not configured")
	}

	// TODO: Fix RefreshToken signature - currently returns interface{}
	// Placeholder implementation
	r.logger.InfoContext(ctx, "token refresh requested",
		"provider_id", providerID,
	)

	return &model.Account{}, nil
}

// DeleteModel deletes a model from the registry
func (r *mutationResolver) DeleteModel(ctx context.Context, id string) (bool, error) {
	if r.models == nil {
		return false, fmt.Errorf("model store not configured")
	}
	// TODO: Implement DeleteModel in ModelStore interface
	r.logger.InfoContext(ctx, "model deletion requested", "model_id", id)
	return true, nil
}

// UpdateModel updates a model in the registry
func (r *mutationResolver) UpdateModel(ctx context.Context, id string, input model.UpdateModelInput) (*model.Model, error) {
	if r.models == nil {
		return nil, fmt.Errorf("model store not configured")
	}
	// TODO: Implement UpdateModel in ModelStore interface
	r.logger.InfoContext(ctx, "model update requested", "model_id", id)
	return &model.Model{}, nil
}

