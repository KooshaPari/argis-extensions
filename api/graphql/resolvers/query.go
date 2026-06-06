package resolvers

import (
	"context"
	"fmt"

	"github.com/kooshapari/bifrost-extensions/api/graphql/model"
)

type queryResolver struct{ *Resolver }

// Models returns all models
func (r *queryResolver) Models(ctx context.Context) ([]*model.Model, error) {
	if r.models == nil {
		return []*model.Model{}, nil
	}

	// TODO: Fix filter interface - currently ModelFilter not fully defined
	modelsData, err := r.models.ListModels(ctx, nil)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to list models", "error", err)
		return nil, err
	}

	// modelsData is a slice of interface{}
	var models []*model.Model
	if modelsData != nil {
		// Convert []interface{} to []*model.Model
		for _, m := range modelsData {
			if modelObj, ok := m.(*model.Model); ok {
				models = append(models, modelObj)
			}
		}
	}

	return models, nil
}

// Model returns a single model by ID
func (r *queryResolver) Model(ctx context.Context, id string) (*model.Model, error) {
	if r.models == nil {
		return nil, fmt.Errorf("model store not configured")
	}
	modelInterface, err := r.models.GetModel(ctx, id)
	if err != nil {
		return nil, err
	}
	m, ok := modelInterface.(*model.Model)
	if !ok {
		return nil, fmt.Errorf("failed to cast model")
	}
	return m, nil
}

// Providers returns all providers
func (r *queryResolver) Providers(ctx context.Context) ([]*model.Provider, error) {
	// TODO: Implement provider listing - ProviderStore interface incomplete
	return []*model.Provider{}, nil
}

// Provider returns a single provider by ID
func (r *queryResolver) Provider(ctx context.Context, id string) (*model.Provider, error) {
	if r.providers == nil {
		return nil, fmt.Errorf("provider store not configured")
	}
	// TODO: Implement GetProvider - not yet defined in ProviderStore
	return nil, fmt.Errorf("GetProvider not implemented")
}

// Benchmarks returns benchmark results with filtering
func (r *queryResolver) Benchmarks(ctx context.Context, first *int, after *string, filter interface{}) (*model.BenchmarkConnection, error) {
	// TODO: Implement benchmark listing - BenchmarkStore interface incomplete
	return &model.BenchmarkConnection{
		Nodes:      []*model.Benchmark{},
		TotalCount: 0,
		PageInfo:   &model.PageInfo{},
	}, nil
}

// Benchmark returns a single benchmark by ID
func (r *queryResolver) Benchmark(ctx context.Context, id string) (*model.Benchmark, error) {
	// TODO: Implement GetBenchmark - not yet defined in BenchmarkStore
	return nil, fmt.Errorf("GetBenchmark not implemented")
}

// Usage returns usage analytics
func (r *queryResolver) Usage(ctx context.Context, timeframe interface{}, groupBy interface{}, filters interface{}) (*model.UsageReport, error) {
	// TODO: Implement usage reporting - UsageStore interface incomplete
	return nil, fmt.Errorf("Usage not implemented")
}

// RoutingHistory returns routing decisions history
func (r *queryResolver) RoutingHistory(ctx context.Context, first *int, after *string, filter interface{}) (*model.RoutingHistoryConnection, error) {
	// TODO: Implement routing history - RoutingStore interface incomplete
	return &model.RoutingHistoryConnection{
		Nodes:      []*model.RoutingHistory{},
		TotalCount: 0,
		PageInfo: &model.PageInfo{
			HasNextPage:     false,
			HasPreviousPage: after != nil,
		},
	}, nil
}

// Policies returns policies with filtering
func (r *queryResolver) Policies(ctx context.Context, policyType interface{}, active *bool) ([]*model.Policy, error) {
	// TODO: Implement policy listing - PolicyStore interface incomplete
	return []*model.Policy{}, nil
}

// Policy returns a single policy by ID
func (r *queryResolver) Policy(ctx context.Context, id string) (*model.Policy, error) {
	// TODO: Implement GetPolicy - not yet defined in PolicyStore
	return nil, fmt.Errorf("GetPolicy not implemented")
}

// Health returns health status information
func (r *queryResolver) Health(ctx context.Context) (*model.HealthStatus, error) {
	return &model.HealthStatus{
		Status: "healthy",
	}, nil
}
