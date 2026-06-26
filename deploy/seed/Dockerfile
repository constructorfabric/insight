# One-shot image for the sample-data seed script.
#
# Re-built only when requirements.txt changes — the script itself is
# bind-mounted at runtime so iterating on seed.py doesn't trigger an
# image rebuild.

FROM python:3.13-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

WORKDIR /app

COPY requirements.txt .
RUN pip install -r requirements.txt

# Source is bind-mounted at runtime (see docker-compose.yml seed-sample
# service). The COPY lines below are a fallback for `docker build &&
# docker run` without a volume — exercised only by ad-hoc inspection,
# so the `generators/` package must come along or `silver.py`'s
# `from generators import ...` fails at import time.
COPY *.py ./
COPY generators ./generators/

ENTRYPOINT ["python", "seed.py"]
CMD ["all"]
