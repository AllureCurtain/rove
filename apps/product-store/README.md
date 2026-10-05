# rove-product-store

## Responsibility

API-global product control-plane state:

- product contracts: `/product/*` request/response types, `ProductStore` trait, error codes, limits
- `SqliteProductStore`: `product.sqlite` schema migrations and repository
- attachment payload path rules shared with the API byte routes
- bundled model pricing for product cost estimates

## Non-responsibility

Does **not** own HTTP routes, SSE, transcript projection, or canonical runtime
events. `rove-api` re-exports this crate under `rove_api::product`.

## Local dependencies

```text
rove-models
rove-runtime
rove-app-bootstrap
```
