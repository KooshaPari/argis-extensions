// Package resolvers implements GraphQL resolvers for the Bifrost API.
package resolvers

import (
	"context"
	"log/slog"
	"sync"

	"github.com/kooshapari/bifrost-extensions/api/graphql/gen"
	"github.com/kooshapari/bifrost-extensions/api/graphql/model"
	"github.com/kooshapari/bifrost-extensions/db"
)

// Store interface definitions for GraphQL resolvers
type ModelFilter struct{}

type ModelStore interface {
	GetModel(ctx context.Context, id string) (interface{}, error)
	ListModels(ctx context.Context, filter *ModelFilter) ([]interface{}, error)
	CreateModel(ctx context.Context, data interface{}) (interface{}, error)
	UpdateModelStatus(ctx context.Context, id string, available bool) (interface{}, error)
}

type ProviderStore interface {
	RefreshToken(ctx context.Context, id string) error
}

type BenchmarkStore interface {
	CreateBenchmark(ctx context.Context, data interface{}) (interface{}, error)
}

type UsageStore interface{}

type RoutingStore interface{}

type PolicyStore interface {
	UpdatePolicy(ctx context.Context, id string, data interface{}) (interface{}, error)
	ActivatePolicy(ctx context.Context, id string) (interface{}, error)
	DeactivatePolicy(ctx context.Context, id string) (interface{}, error)
}

// Resolver is the root resolver that provides access to all sub-resolvers.
type Resolver struct {
	db     *db.DB
	logger *slog.Logger

	// Store interfaces for data access
	models     ModelStore
	providers  ProviderStore
	benchmarks BenchmarkStore
	usage      UsageStore
	routing    RoutingStore
	policies   PolicyStore

	// Subscription management
	mu              sync.RWMutex
	healthSubs      map[string]chan *model.ProviderHealthEvent
	availabilitySubs map[string]chan *model.ModelAvailabilityEvent
	routingSubs     map[string]chan *model.RoutingEvent
	usageSubs       map[string]chan *model.UsageUpdate
}

// NewResolver creates a new root resolver.
func NewResolver(database *db.DB, opts ...ResolverOption) *Resolver {
	r := &Resolver{
		db:              database,
		logger:          slog.Default(),
		healthSubs:      make(map[string]chan *model.ProviderHealthEvent),
		availabilitySubs: make(map[string]chan *model.ModelAvailabilityEvent),
		routingSubs:     make(map[string]chan *model.RoutingEvent),
		usageSubs:       make(map[string]chan *model.UsageUpdate),
	}
	for _, opt := range opts {
		opt(r)
	}
	return r
}

// ResolverOption configures the resolver
type ResolverOption func(*Resolver)

// WithLogger sets the logger
func WithLogger(l *slog.Logger) ResolverOption {
	return func(r *Resolver) { r.logger = l }
}

// WithModelStore sets the model store
func WithModelStore(s ModelStore) ResolverOption {
	return func(r *Resolver) { r.models = s }
}

// WithProviderStore sets the provider store
func WithProviderStore(s ProviderStore) ResolverOption {
	return func(r *Resolver) { r.providers = s }
}

// WithBenchmarkStore sets the benchmark store
func WithBenchmarkStore(s BenchmarkStore) ResolverOption {
	return func(r *Resolver) { r.benchmarks = s }
}

// WithUsageStore sets the usage store
func WithUsageStore(s UsageStore) ResolverOption {
	return func(r *Resolver) { r.usage = s }
}

// WithRoutingStore sets the routing store
func WithRoutingStore(s RoutingStore) ResolverOption {
	return func(r *Resolver) { r.routing = s }
}

// WithPolicyStore sets the policy store
func WithPolicyStore(s PolicyStore) ResolverOption {
	return func(r *Resolver) { r.policies = s }
}

// Query returns the QueryResolver implementation.
func (r *Resolver) Query() gen.QueryResolver {
	return &queryResolver{r}
}

// Mutation returns the MutationResolver implementation.
func (r *Resolver) Mutation() gen.MutationResolver {
	return &mutationResolver{r}
}

// Subscription returns the SubscriptionResolver implementation.
func (r *Resolver) Subscription() gen.SubscriptionResolver {
	return &subscriptionResolver{r}
}
