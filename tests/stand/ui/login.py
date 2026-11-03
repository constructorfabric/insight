"""Select the password form without changing the OIDC request's other fields."""

from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit


def password_form_url(authorize_url: str) -> str:
    parts = urlsplit(authorize_url)
    query = [
        (key, value)
        for key, value in parse_qsl(parts.query, keep_blank_values=True)
        if key != "kc_idp_hint"
    ]
    query.append(("kc_idp_hint", ""))
    return urlunsplit(parts._replace(query=urlencode(query)))
