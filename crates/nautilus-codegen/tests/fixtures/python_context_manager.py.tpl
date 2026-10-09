import asyncio
from unittest.mock import AsyncMock, Mock, patch

from generated import Nautilus


__ASYNC__def check_context_manager() -> None:
    for fail in (False, True):
        original = Nautilus()
        expected = RuntimeError("context body failed")
        with (
            patch.object(Nautilus, "connect", new_callable=__MOCK__) as connect,
            patch.object(Nautilus, "disconnect", new_callable=__MOCK__) as disconnect,
        ):
            try:
                __ASYNC__with original as client:
                    assert client is original
                    assert client.order is original.order
                    if fail:
                        raise expected
            except RuntimeError as error:
                assert fail and error is expected
            else:
                assert not fail
            connect.__ASSERT_ONCE__()
            disconnect.__ASSERT_ONCE__()


__RUN__
