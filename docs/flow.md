# KnockturnAllee Payment flow

Current version supports customer-initiated payments, invoice support will be added in the next revision.

## Create order
Merchant's backend sends a request to KA to register an order, uisng API token or user/password. It requires somes additional coding on a merchant's side and must be done before a user will be redirected to an order page.

```
POST /merchants/{merchantId}

{
	"order_id": "xyz",
	"amount" : {
		"amount": 1000,
		"currency: "USD"
		},
	confirmations: 10,
	email: "user@domain.com"
}
```
Amount is in minimal units for the currency (cents, satoshi, nanogrins). Email is optional and used to send a notification about status changes of the payment.

In case of success KA returns 201. KA calculates amount in grins and expiration time, set status `UNPAID` and saves order in DB. KA sends an email to a customer if an address is provided. 

## Get order status
After that a customer is redirected to an order status page. This request may be made multiple times to get the current state of the order.
```
GET /merchants/{merchantId}/orders/{orderId}
```

KA returns a human-readable HTML page with a payment URL (same as the URL of the order page), amount in grins, status of the order (initially UNPAID) and additional information. e.g. expiration time for this invoice.

## Payment
Customers initiates a payment using her grin wallet to the URL from the order status page
```
POST  /merchants/{merchantId}/orders/{orderId}

{Grin Tx slate}
```

KA acts as a proxy for Grin wallet. Before forwarding a request it checks that order exists, otherwise returns 404.
It checks that amount equals to the order amount and status is UNPAID or REJECTED.  After receiving a response from the wallet KA updates status of the payment (RECEIVED or REJECTED).

## Confirmation
KA polls the wallet to get updates reg the on-chain status of transaction, this information is available via `Get order status` request. When the tx gets a required number of confirmations KA sends a request to `callbackUrl` configured for the merchant and sends an email to the customer.

## Withdrawal
Merchant is able to configure different policies of withdrawal:

- Immediate automatic
- Daily automatic
- Manual

For the first 2 the merchan't wallet must be available as HTTPS endpoint. For manual withdrawal a pure HTTPS client mode will be supported, so a merchant will be able to send a payment request, get a slate, sign it and send back without having a listening wallet. 
