package resolvers

import (
	"context"
	"fmt"

	"github.com/kooshapari/bifrost-extensions/api/graphql/model"
)

type queryResolver struct{ *Resolver }

// Models returns all models with pagination and filtering
func (r *queryResolver) Models(ctx context.Context, first *int, after *string, filter *model.ModelFilter) (*model.ModelConnection, error) {
	if r.models == nil {
		return &model.ModelConnection{
			Nodes:      []*model.Model{},
			TotalCount: 0,
			PageInfo:   &model.PageInfo{},
		}, nil
	}

	lim := 100
	if first != nil {
		lim = *first
	}

	internalFilter := ModelFilter{
		Limit: lim,
	}
	if filter != nil {
		if len(filter.Providers) > 0 {
			internalFilter.Provider = &filter.Providers[0]
		}
		internalFilter.Capabilities = filter.Capabilities
		internalFilter.Available = filter.Available
	}

	models, total, err := r.models.ListModels(ctx, internalFilter)
	if err != nil {
		r.logger.ErrorContext(ctx, "failed to list models", "error", err)
		return nil, err
	}

	hasNext := len(models) < total

	return &model.ModelConnection{
		Nodes:      models,
		TotalCount: total,
		PageInfo: &model.PageInfo{
			HasNextPage:     hasNext,
			HasPreviousPage: after != nil,
		},
	}, nil
}

// Model returns a single model by ID
func (r *queryResolver) Model(ctx context.Context, id string) (*model.Model, error) {
	if r.models == nil {
		return nil, fmt.Errorf("model store not configured")
	}
	return r.models.GetModel(ctx, id)
}

// Providers returns all providers
func (r *queryResolver) Providers(ctx context.Context, status *model.ProviderStatus) ([]*model.Provider, error) {
	if r.providers == nil {
		return []*model.Provider{}, nil
	}
	providers, err := r.providers.ListProviders(ctx)
	if err != nil {
		return nil, err
	}
	// Filter by status if provided
	if status != nil {
		var filtered []*model.Provider
		for _, p := range providers {
			if p.Status == *status {
				filtered = append(filtered, p)
			}
		}
		return filtered, nil
	}
	return providers, nil
}

// Provider returns a single provider by ID
func (r *queryResolver) Provider(ctx context.Context, id string) (*model.Provider, error) {
	if r.providers == nil {
		return nil, fmt.Errorf("provider store not configured")
	}
	return r.providers.GetProvider(ctx, id)
}

// Benchmarks returns benchmark results with filtering
func (r *queryResolver) Benchmarks(ctx context.Context, first *int, after *string, filter *model.BenchmarkFilter) (*model.BenchmarkConnection, error) {
	if r.benchmarks == nil {
		return &model.BenchmarkConnection{
			Nodes:      []*model.Benchmark{},
			TotalCount: 0,
			PageInfo:   &model.PageInfo{},
		}, nil
	}

	lim := 50
	if first != nil {
		lim = *first
	}

	internalFilter := BenchmarkFilter{Limit: lim}
	if filter != nil {
		internalFilter.Models = filter.ModelIds
	}

	benchmarks, total, err := r.benchmarks.ListBenchmarks(ctx, internalFilter)
	if err != nil {
		return nil, err
	}

	return &model.BenchmarkConnection{
		Nodes:      benchmarks,
		TotalCount: total,
		PageInfo:   &model.PageInfo{HasNextPage: len(benchmarks) == lim},
	}, nil
}

// Benchmark returns a single benchmark by ID
func (r *queryResolver) Benchmark(ctx context.Context, id string) (*model.Benchmark, error) {
	if r.benchmarks == nil {
		return nil, fmt.Errorf("benchmark store not configured")
	}
	return r.benchmarks.GetBenchmark(ctx, id)
}

// Usage returns usage analytics
func (r *queryResolver) Usage(ctx context.Context, timeframe model.Timeframe, groupBy []model.GroupByField, filters *model.UsageFilters) (*model.UsageReport, error) {
	if r.usage == nil {
		return nil, fmt.Errorf("usage store not configured")
	}
	return r.usage.GetUsageReport(ctx, UsageFilter{
		Timeframe: timeframe,
		GroupBy:   groupBy,
		Filters:   filters,
	})
}

// RoutingHistory returns routing decisions history
func (r *queryResolver) RoutingHistory(ctx context.Context, first *int, after *string, filter *model.RoutingFilter) (*model.RoutingHistoryConnection, error) {
	if r.routing == nil {
		return &model.RoutingHistoryConnection{
			Nodes:      []*model.RoutingHistory{},
			TotalCount: 0,
			PageInfo:   &model.PageInfo{},
		}, nil
	}

	lim := 100
	if first != nil {
		lim = *first
	}

	internalFilter := RoutingFilter{Limit: lim}
	if filter != nil {
		internalFilter.SessionID = filter.SessionID
		internalFilter.UserID = filter.UserID
	}

	history, total, err := r.routing.GetRoutingHistory(ctx, internalFilter)
	if err != nil {
		return nil, err
	}

	return &model.RoutingHistoryConnection{
		Nodes:      history,
		TotalCount: total,
		PageInfo: &model.PageInfo{
			HasNextPage:     len(history) < total,
			HasPreviousPage: after != nil,
		},
	}, nil
}

// Policies returns policies with filtering
func (r *queryResolver) Policies(ctx context.Context, policyType *model.PolicyType, active *bool) ([]*model.Policy, error) {
	if r.policies == nil {
		return []*model.Policy{}, nil
	}
	return r.policies.ListPolicies(ctx, PolicyFilter{
		Type:   policyType,
		Active: active,
	})
}

// Policy returns a single policy by ID
func (r *queryResolver) Policy(ctx context.Context, id string) (*model.Policy, error) {
	if r.policies == nil {
		return nil, fmt.Errorf("policy store not configured")
	}
	return r.policies.GetPolicy(ctx, id)
}
