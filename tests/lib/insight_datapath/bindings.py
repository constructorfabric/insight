"""Binding a spec's source accounts to the people it means, through the product.

A spec can state that two accounts are one human (`identity_aliases`) or that an
account belongs to a named person whatever its email says (`identity_accounts`).
Resolution by email cannot reach either: a second address is a different address,
and an account whose profile email was never collected has nothing to match on.

The product's answer to both is an operator decision, and that is what this
applies. The retired rig inserted the journal rows itself, which proved the metric
and not the binding; a correction made through the API takes the same path an
operator takes, and publishes itself to ClickHouse as it lands.

The API addresses an account by the hashed source id the connectors mint, so the
raw id a spec writes is hashed by the warehouse with the connectors' own
expression rather than by a second implementation here.
"""

from __future__ import annotations

import logging

from insight_stand.api import ApiClient, identity_path

from insight_datapath import clickhouse as ch
from insight_datapath.fixture_loader import IdentityAccount
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.bindings")

#: The reserved person a bot's or a service account's work attributes to.
EXCLUDED = "excluded"


class BindingError(RuntimeError):
    """The product declined an operator decision a spec depends on."""


def hashed_source_id(cfg: InstanceConfig, raw: str) -> str:
    """The connector instance id the warehouse mints from a bronze `source_id`."""
    rows = ch.query(
        cfg,
        f"SELECT toString(toUUID(UUIDNumToString(sipHash128('{raw}')))) ",
    )
    return str(rows[0][0])


def wire_account(source_type: str, source_id: str, account_id: str) -> dict[str, str]:
    """An account as the resolution API names it."""
    return {"source": source_type, "source_id": source_id, "id": account_id}


class Bindings:
    """Applies a spec's declared account decisions as an operator."""

    def __init__(self, cfg: InstanceConfig, *, client: ApiClient) -> None:
        self.cfg = cfg
        self._client = client

    def apply(
        self,
        accounts: list[IdentityAccount],
        aliases: dict[str, list[str]],
        person_ids: dict[str, str],
    ) -> int:
        """Bind what the spec declared; returns how many decisions were made."""
        decisions = [
            *self._from_accounts(accounts, person_ids),
            *self._from_aliases(aliases, person_ids),
        ]
        bindings = [(account, person) for account, person in decisions if person != EXCLUDED]
        excluded = [account for account, person in decisions if person == EXCLUDED]

        if bindings:
            self._bind(bindings)
        for account in excluded:
            self._exclude(account)
        if decisions:
            LOG.info("operator decisions applied: %d", len(decisions))
        return len(decisions)

    def _from_accounts(
        self, accounts: list[IdentityAccount], person_ids: dict[str, str]
    ) -> list[tuple[dict[str, str], str]]:
        out = []
        for entry in accounts:
            person = (
                EXCLUDED if entry.person == EXCLUDED else self._person(entry.person, person_ids)
            )
            account = wire_account(
                entry.source_type,
                hashed_source_id(self.cfg, entry.source_id),
                entry.account_id,
            )
            out.append((account, person))
        return out

    def _from_aliases(
        self, aliases: dict[str, list[str]], person_ids: dict[str, str]
    ) -> list[tuple[dict[str, str], str]]:
        out = []
        for canonical, alternates in aliases.items():
            person = self._person(canonical, person_ids)
            for alias in alternates:
                found = self._accounts_claiming(alias)
                if not found:
                    raise BindingError(
                        f"no account in the identity inputs claims {alias}, so there is nothing "
                        f"to bind to {canonical}. An alias names a second account of one person, "
                        "and the spec has to seed that account's own activity for it to exist."
                    )
                out.extend((account, person) for account in found)
        return out

    def _person(self, email: str, person_ids: dict[str, str]) -> str:
        person = person_ids.get(email.strip().lower())
        if not person:
            raise BindingError(
                f"the spec binds an account to {email}, whom identity resolved no person for. "
                "A person a binding names has to be one the spec seeds an employment record for."
            )
        return person

    def _accounts_claiming(self, email: str) -> list[dict[str, str]]:
        rows = ch.query(
            self.cfg,
            f"""
            SELECT DISTINCT
                insight_source_type,
                toString(insight_source_id),
                lower(trimBoth(source_account_id))
            FROM identity.identity_inputs
            WHERE value_type = 'email'
              AND operation_type = 'UPSERT'
              AND lower(trimBoth(value)) = '{email.strip().lower()}'
              AND coalesce(source_account_id, '') != ''
            """,
        )
        return [
            wire_account(str(source), str(source_id), str(account))
            for source, source_id, account in rows
        ]

    def _bind(self, bindings: list[tuple[dict[str, str], str]]) -> None:
        response = self._client.post(
            identity_path("/v1/resolution/bind"),
            json_body={
                "bindings": [
                    {"account": account, "person_id": person} for account, person in bindings
                ],
                "comment": "datapath spec binding",
            },
        )
        if response.status_code != 200:
            raise BindingError(f"bind refused with {response.status_code}: {response.text[:500]}")

    def _exclude(self, account: dict[str, str]) -> None:
        response = self._client.post(
            identity_path("/v1/resolution/exclude"),
            json_body={"account": account, "comment": "datapath spec exclusion"},
        )
        if response.status_code != 200:
            raise BindingError(
                f"exclude refused with {response.status_code} for {account}: {response.text[:500]}"
            )
